//! Private JSON-lines protocol over inherited stdin/stdout; no listening socket.
//! The per-user desktop host owns the tray and OS events; sync and credentials stay here.
use anyhow::{bail, Context, Result};
use filenode::{
    auth::{self, Authentication},
    config::{AuthMode, Config},
    group::SyncGroup,
    jmap::{Jmap, Remote},
    runtime::{Policy, PowerState, Scope},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone)]
struct Output(Arc<Mutex<std::io::Stdout>>);
impl Output {
    fn send(&self, event: Value) {
        if let Ok(mut output) = self.0.lock() {
            let _ = serde_json::to_writer(&mut *output, &event);
            let _ = output.write_all(b"\n");
            let _ = output.flush();
        }
    }
}

pub fn run(path: PathBuf, gui: Option<PathBuf>, external_autostart: bool) -> Result<()> {
    let mut config = if path.exists() {
        Config::load(&path)?
    } else {
        Config::default()
    };
    config.resolve_folder_paths()?;
    if config.autostart && !external_autostart {
        if let Some(gui) = &gui {
            crate::autostart::set_executable(true, &path, gui)?;
        }
    }
    let output = Output(Arc::new(Mutex::new(std::io::stdout())));
    let power = Arc::new(PowerState::default());
    let running = Arc::new(AtomicBool::new(false));
    let mut policy = Policy::new(&config, Arc::new(AtomicBool::new(false)), power.clone());
    let immediate = Arc::new(AtomicBool::new(false));
    let mut worker: Option<thread::JoinHandle<()>> = None;
    output.send(json!({"event":"ready","protocol":4,"config":config}));
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        let mut line = String::new();
        let size = reader.by_ref().take(1_048_577).read_line(&mut line)?;
        if size == 0 {
            break;
        }
        if size > 1_048_576 {
            line.zeroize();
            bail!("Desktop command exceeds size limit");
        }
        let parsed = serde_json::from_str::<Value>(&line);
        line.zeroize();
        let mut command = match parsed {
            Ok(value) => value,
            Err(error) => {
                output.send(json!({"event":"error","details":error.to_string()}));
                continue;
            }
        };
        let kind = command["command"].as_str().unwrap_or("").to_owned();
        if kind == "quit" {
            break;
        }
        let result: Result<()> = (|| {
            match kind.as_str() {
                "configure" => {
                    let mut next: Config = serde_json::from_value(command["config"].take())?;
                    next.resolve_folder_paths()?;
                    if next.sync_folders != config.sync_folders {
                        next.normalize_folders(&config)?;
                    }
                    if next.upload_limit_kib > 1_000_000
                        || next.download_limit_kib > 1_000_000
                        || next.file_settle_seconds > 60
                    {
                        bail!("Invalid transfer or settling limits");
                    }
                    if running.load(Ordering::Relaxed)
                        && (next.server != config.server
                            || next.username != config.username
                            || next.account_id != config.account_id
                            || next.sync_folders != config.sync_folders)
                    {
                        bail!("Pause synchronization before changing the account or folders");
                    }
                    if running.load(Ordering::Relaxed)
                        && (next.auth_mode != config.auth_mode
                            || next.oauth_issuer != config.oauth_issuer
                            || next.oauth_client_id != config.oauth_client_id
                            || next.oauth_scopes != config.oauth_scopes
                            || next.oauth_redirect_port != config.oauth_redirect_port)
                    {
                        bail!("Pause synchronization before changing authentication settings");
                    }
                    if next.autostart != config.autostart && !external_autostart {
                        if let Some(gui) = &gui {
                            crate::autostart::set_executable(next.autostart, &path, gui)?;
                        } else {
                            crate::autostart::set(next.autostart, &path)?;
                        }
                    }
                    let account_changed = next.credential_key() != config.credential_key()
                        || next.auth_mode != config.auth_mode
                        || next.oauth_credential_key() != config.oauth_credential_key()
                        || next.account_id != config.account_id;
                    next.save(&path)?;
                    config = next;
                    if account_changed {
                        output.send(json!({"event":"account","state":"signed_out"}));
                    }
                    policy.configure(&config);
                    output.send(json!({"event":"configured","config":config}));
                }
                "connect" => {
                    if running.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    if let Some(previous) = worker.take() {
                        let _ = previous.join();
                    }
                    config.validate_account()?;
                    let entry = keyring::Entry::new("FileNode", &config.credential_key())?;
                    let supplied =
                        Zeroizing::new(command["password"].as_str().unwrap_or("").to_owned());
                    if let Some(Value::String(password)) = command.get_mut("password") {
                        password.zeroize();
                    }
                    command["password"] = Value::Null;
                    let basic = if config.auth_mode == AuthMode::AppPassword {
                        Some(Authentication::basic(
                            &config.username,
                            if !supplied.is_empty() {
                                supplied.to_string()
                            } else {
                                entry
                                    .get_password()
                                    .context("App password unavailable in OS credential store")?
                            },
                        ))
                    } else {
                        None
                    };
                    let interactive = command["interactive"].as_bool().unwrap_or(false);
                    config.save(&path)?;
                    policy = Policy::new(&config, Arc::new(AtomicBool::new(false)), power.clone());
                    let (settings, policy, running, immediate, output) = (
                        config.clone(),
                        policy.clone(),
                        running.clone(),
                        immediate.clone(),
                        output.clone(),
                    );
                    running.store(true, Ordering::Relaxed);
                    output.send(json!({"event":"account","state":"checking"}));
                    output.send(json!({"event":"running","value":true,"config":settings}));
                    worker = Some(thread::spawn(move || {
                        let _scope = Scope::enter(policy.clone());
                        let activity_output = output.clone();
                        let _activity = filenode::activity::Scope::observe(move |event| {
                            if let Ok(value) = serde_json::to_value(event) {
                                activity_output.send(value);
                            }
                        });
                        let mut password_to_save = if settings.auth_mode == AuthMode::AppPassword
                            && !supplied.is_empty()
                        {
                            Some(supplied)
                        } else {
                            None
                        };
                        let mut group = match SyncGroup::new(&settings) {
                            Ok(group) => group,
                            Err(error) => {
                                output.send(json!({"event":"account","state":"failed"}));
                                output
                                    .send(json!({"event":"error","details":format!("{error:#}")}));
                                running.store(false, Ordering::Relaxed);
                                output.send(json!({"event":"running","value":false}));
                                return;
                            }
                        };
                        let mut connection = None;
                        let mut account_verified = false;
                        let mut revision = policy.power.revision.load(Ordering::SeqCst);
                        let mut backoff = 5;
                        let result = (|| -> Result<()> {
                            let authentication = match basic {
                                Some(authentication) => authentication,
                                None => {
                                    output
                                        .send(json!({"event":"status","key":"status.signing_in"}));
                                    auth::browser_authentication(
                                        &settings,
                                        interactive,
                                        &policy.cancel,
                                        |url| {
                                            output.send(
                                                json!({"event":"open_browser","url":url.as_str()}),
                                            );
                                        },
                                    )?
                                }
                            };
                            while !policy.cancel.load(Ordering::Relaxed) {
                                if policy.power_paused() && account_verified {
                                    output.send(
                                        json!({"event":"status","key":"status.power_paused"}),
                                    );
                                    while policy.power_paused()
                                        && !policy.cancel.load(Ordering::Relaxed)
                                    {
                                        thread::sleep(Duration::from_millis(250));
                                    }
                                    group.invalidate();
                                    connection = None;
                                    if policy.cancel.load(Ordering::Relaxed) {
                                        break;
                                    }
                                }
                                let current = policy.power.revision.load(Ordering::SeqCst);
                                if current != revision {
                                    revision = current;
                                    group.invalidate();
                                    connection = None;
                                }
                                if immediate.swap(false, Ordering::Relaxed) {
                                    group.invalidate();
                                }
                                if !group.ready() && connection.is_some() {
                                    thread::sleep(Duration::from_millis(250));
                                    continue;
                                }
                                let result = (|| {
                                    if connection.is_none() {
                                        output.send(
                                            json!({"event":"status","key":"status.connecting"}),
                                        );
                                        let mut remote = Jmap::connect_authenticated(
                                            &settings,
                                            authentication.clone(),
                                            policy.cancel.clone(),
                                        )?;
                                        // Check authenticated API access before claiming sign-in;
                                        // this is a state-only request, not a file listing or scan.
                                        remote
                                            .current_state()
                                            .context("Verify file-account access")?;
                                        remote.set_policy(policy.clone());
                                        if let Some(password) = password_to_save.as_ref() {
                                            keyring::Entry::new(
                                                "FileNode",
                                                &settings.credential_key(),
                                            )?
                                            .set_password(password)?;
                                            password_to_save = None;
                                        }
                                        account_verified = true;
                                        output.send(json!({"event":"account","state":"signed_in","trash_url":remote.web_trash_url()}));
                                        connection = Some(remote);
                                    }
                                    let mut log = |message| {
                                        output.send(json!({"event":"activity","message":message}))
                                    };
                                    group.poll(
                                        connection.as_mut().unwrap(),
                                        &policy.cancel,
                                        &mut log,
                                    )
                                })();
                                match result {
                                    Ok(report) => {
                                        backoff = 5;
                                        if report.failed_folders > 0 {
                                            connection = None;
                                        }
                                        output.send(json!({"event":"report","report":report}));
                                        if settings.sync_folders.as_ref().is_some_and(Vec::is_empty)
                                        {
                                            break;
                                        }
                                    }
                                    Err(_) if policy.cancel.load(Ordering::Relaxed) => break,
                                    Err(_) if policy.power_paused() && account_verified => {
                                        connection = None;
                                        continue;
                                    }
                                    Err(error)
                                        if error
                                            .downcast_ref::<auth::SignInRequired>()
                                            .is_some() =>
                                    {
                                        return Err(error)
                                    }
                                    Err(error) => {
                                        output.send(
                                            json!({"event":"retry","details":format!("{error:#}")}),
                                        );
                                        connection = None;
                                        group.invalidate();
                                        let until = Instant::now() + Duration::from_secs(backoff);
                                        backoff = (backoff * 2).min(300);
                                        while Instant::now() < until
                                            && !policy.cancel.load(Ordering::Relaxed)
                                            && (!policy.power_paused() || !account_verified)
                                        {
                                            if immediate.swap(false, Ordering::Relaxed) {
                                                break;
                                            }
                                            thread::sleep(Duration::from_millis(250));
                                        }
                                    }
                                }
                            }
                            Ok(())
                        })();
                        if let Err(error) = result {
                            output.send(json!({"event":"account","state":"failed"}));
                            output.send(json!({"event":"error","details":format!("{error:#}")}));
                        }
                        running.store(false, Ordering::Relaxed);
                        output.send(json!({"event":"running","value":false}));
                    }));
                }
                "pause" => policy.cancel.store(true, Ordering::Relaxed),
                "check" => immediate.store(true, Ordering::Relaxed),
                "power" => {
                    power.available.store(
                        command["available"].as_bool().unwrap_or(false),
                        Ordering::Relaxed,
                    );
                    power.set(
                        command["saver"].as_bool().unwrap_or(false),
                        command["sleeping"].as_bool().unwrap_or(false),
                    );
                    output.send(json!({"event":"power","paused":policy.power_paused(),"available":power.available.load(Ordering::Relaxed)}));
                }
                _ => bail!("Unsupported desktop command"),
            }
            Ok(())
        })();
        if let Some(Value::String(password)) = command.get_mut("password") {
            password.zeroize();
        }
        if let Err(error) = result {
            output.send(json!({"event":"error","details":format!("{error:#}")}));
        }
    }
    policy.cancel.store(true, Ordering::Relaxed);
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    Ok(())
}
