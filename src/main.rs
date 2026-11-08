use anyhow::{Context, Result};
use clap::Parser;
use filenode::{
    config::Config,
    engine, filesystem,
    group::SyncGroup,
    i18n::{Language, Message},
    jmap::{Jmap, Remote},
    planner,
    store::Store,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[cfg(feature = "desktop")]
mod autostart;
#[cfg(feature = "desktop")]
mod service;

#[derive(Parser)]
#[command(version, about = "FileNode — JMAP Files for Windows and Linux")]
struct Args {
    /// Run without the desktop interface
    #[arg(long)]
    headless: bool,
    /// Synchronize once and exit
    #[arg(long)]
    once: bool,
    /// Preview the plan without changing user files or remote objects
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Presentation language (technical diagnostics and CLI help remain English)
    #[arg(long, value_enum)]
    language: Option<Language>,
    /// Start the desktop in the notification area
    #[arg(long)]
    background: bool,
    /// Open the settings window immediately
    #[arg(long)]
    settings: bool,
    /// Disable the tray for debugging or desktops without a status notifier host
    #[arg(long)]
    no_tray: bool,
    /// Internal JSON-lines worker used by the desktop frontends
    #[arg(long)]
    service: bool,
    #[arg(long, hide = true)]
    gui_executable: Option<PathBuf>,
    /// The packaged frontend owns its operating-system startup registration
    #[arg(long, hide = true)]
    external_autostart: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = args.config.unwrap_or(Config::default_path()?);
    if args.service {
        #[cfg(feature = "desktop")]
        return service::run(path, args.gui_executable, args.external_autostart);
        #[cfg(not(feature = "desktop"))]
        anyhow::bail!("The desktop service requires the desktop feature");
    }
    if !args.headless && !args.once && !args.dry_run {
        let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
            "filenode-ui.exe"
        } else {
            "filenode-ui"
        });
        let mut command = std::process::Command::new(executable);
        if let Some(language) = args.language {
            let mut config = if path.exists() {
                Config::load(&path)?
            } else {
                Config::default()
            };
            config.language = language;
            config.save(&path)?;
        }
        command.arg("--config").arg(&path);
        if args.background {
            command.arg("--background");
        }
        if args.settings {
            command.arg("--settings");
        }
        if args.no_tray {
            command.arg("--no-tray");
        }
        return command.spawn().map(|_| ()).context(
            "Desktop frontend is missing; build apps/windows or apps/linux, or use --headless",
        );
    }
    let mut config = Config::load(&path)
        .with_context(|| format!("Configuration is missing: {}", path.display()))?;
    if let Some(language) = args.language {
        config.language = language;
    }
    let language = config.language.resolve();
    config.validate()?;
    let cancel = Arc::new(AtomicBool::new(false));
    let authentication = if config.auth_mode == filenode::config::AuthMode::Browser {
        filenode::auth::browser_authentication(&config, false, &cancel, |_| {})?
    } else {
        filenode::auth::Authentication::basic(&config.username, password(&config)?)
    };
    let mut remote = Jmap::connect_authenticated(&config, authentication, cancel.clone())?;
    if args.dry_run {
        let snapshot = remote.snapshot(None)?;
        let mut plans = Vec::new();
        for folder in config.folder_configs()? {
            folder.validate()?;
            let store = Store::open(&folder)?;
            let local = filesystem::scan(&folder.local_folder, folder.max_file_bytes)?;
            let root = match store.get_meta::<String>("remote_root_id")? {
                Some(root) => {
                    if !snapshot.nodes.iter().any(|node| {
                        node.id == root && node.kind == filenode::model::Kind::Directory
                    }) {
                        anyhow::bail!("Remote sync folder is missing; dry-run blocked");
                    }
                    Some(root)
                }
                None => engine::find_folder_path(&snapshot, &folder.remote_folder_path()?)?,
            };
            let tree = match root {
                Some(root) => filenode::jmap::scoped_tree(&snapshot, Some(&root))?,
                None => Default::default(),
            };
            plans.push(serde_json::json!({"local_folder":folder.local_folder,"remote_path":folder.remote_folder_path()?,"actions":planner::plan(&local,&tree,&store.baselines()?,&folder)?}));
        }
        println!("{}", serde_json::to_string_pretty(&plans)?);
        return Ok(());
    }
    let mut log = |line: Message| {
        println!(
            "{} {}",
            chrono::Local::now().format("%H:%M:%S"),
            line.render(language)
        )
    };
    let once_deadline = std::time::Instant::now()
        + Duration::from_secs(config.file_settle_seconds.saturating_mul(2) + 30);
    let mut group = SyncGroup::new(&config)?;
    loop {
        let report = group.poll(&mut remote, &cancel, &mut log)?;
        println!(
            "{}",
            Message::new("cli.report")
                .with("uploaded", report.uploaded)
                .with("downloaded", report.downloaded)
                .with("conflicts", report.conflicts)
                .with("recycled", report.recycled)
                .with("blocked", report.blocked)
                .with("renamed", report.renamed)
                .render(language)
        );
        if report.failed_folders > 0 && args.once {
            anyhow::bail!("One or more folders could not synchronize; see the folder errors above");
        }
        if report.needs_rescan {
            continue;
        }
        if args.once {
            if report.deferred > 0 {
                if std::time::Instant::now() >= once_deadline {
                    anyhow::bail!("Files remain busy or changing; use the desktop worker for continued retries");
                }
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        while !group.ready() && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}

fn password(config: &Config) -> Result<String> {
    if let Ok(value) = std::env::var("FILENODE_PASSWORD") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    #[cfg(feature = "desktop")]
    {
        keyring::Entry::new("FileNode", &config.credential_key())?
            .get_password()
            .context("App password is missing from the operating system credential store")
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = config;
        anyhow::bail!("FILENODE_PASSWORD is not set")
    }
}
