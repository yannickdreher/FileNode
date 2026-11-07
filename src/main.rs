#![cfg_attr(
    all(windows, feature = "desktop", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use anyhow::{Context, Result};
use clap::Parser;
use noritec_drive::{
    config::Config,
    engine, filesystem,
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
mod tray;
#[cfg(feature = "desktop")]
mod ui;

#[derive(Parser)]
#[command(version, about = "Noritec Drive — JMAP Files for Windows and Linux")]
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
}

fn main() -> Result<()> {
    // Desktop releases should not leave a console window beside the tray icon.
    // CLI invocations still attach to the caller's console before producing output.
    #[cfg(all(windows, feature = "desktop", not(debug_assertions)))]
    if std::env::args().any(|arg| {
        matches!(
            arg.as_str(),
            "--headless" | "--once" | "--dry-run" | "--help" | "-h" | "--version" | "-V"
        )
    }) {
        unsafe {
            use windows_sys::Win32::{
                Foundation::INVALID_HANDLE_VALUE,
                System::Console::{
                    AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
                },
            };
            let output = GetStdHandle(STD_OUTPUT_HANDLE);
            if output.is_null() || output == INVALID_HANDLE_VALUE {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
    let args = Args::parse();
    let path = args.config.unwrap_or(Config::default_path()?);
    if !args.headless && !args.once && !args.dry_run {
        #[cfg(feature = "desktop")]
        return ui::run(
            path,
            args.language,
            args.background,
            args.settings,
            args.no_tray,
        );
        #[cfg(not(feature = "desktop"))]
        anyhow::bail!("This build has no desktop interface. Use --headless or --once.");
    }
    let mut config = Config::load(&path)
        .with_context(|| format!("Configuration is missing: {}", path.display()))?;
    if let Some(language) = args.language {
        config.language = language;
    }
    let language = config.language.resolve();
    config.validate()?;
    let mut store = Store::open(&config)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut remote = Jmap::connect(&config, password(&config)?, cancel.clone())?;
    let mut scanner =
        noritec_drive::scanner::LocalScanner::new(&config.local_folder, config.full_scan_seconds);
    if args.dry_run {
        let local = filesystem::scan(&config.local_folder, config.max_file_bytes)?;
        let snapshot = remote.snapshot(None)?;
        let root = store.get_meta::<String>("remote_root_id")?.or_else(|| {
            snapshot
                .nodes
                .iter()
                .find(|n| n.parent_id.is_none() && n.name == config.remote_folder)
                .map(|n| n.id.clone())
        });
        let tree = match root {
            Some(root) => noritec_drive::jmap::scoped_tree(&snapshot, &root)?,
            None => Default::default(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&planner::plan(
                &local,
                &tree,
                &store.baselines()?,
                &config
            )?)?
        );
        return Ok(());
    }
    let mut log = |line: Message| {
        println!(
            "{} {}",
            chrono::Local::now().format("%H:%M:%S"),
            line.render(language)
        )
    };
    loop {
        let report = engine::sync_with_scanner(
            &config,
            &mut store,
            &mut remote,
            &cancel,
            &mut log,
            &mut scanner,
        )?;
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
        if report.needs_rescan {
            continue;
        }
        if args.once {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(config.interval_seconds);
        while std::time::Instant::now() < deadline && !scanner.ready() {
            scanner.wait(deadline.saturating_duration_since(std::time::Instant::now()));
        }
    }
    Ok(())
}

fn password(config: &Config) -> Result<String> {
    if let Ok(value) = std::env::var("NORITEC_DRIVE_PASSWORD") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    #[cfg(feature = "desktop")]
    {
        keyring::Entry::new("de.noritec.drive", &config.credential_key())?
            .get_password()
            .context("App password is missing from the operating system credential store")
    }
    #[cfg(not(feature = "desktop"))]
    {
        let _ = config;
        anyhow::bail!("NORITEC_DRIVE_PASSWORD is not set")
    }
}
