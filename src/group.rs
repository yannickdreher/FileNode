//! Serial multi-folder orchestration. Stores, watchers and retry deadlines are per root;
//! credentials, the remote connection and transfer budgets belong to the account.
use crate::{
    auth,
    config::Config,
    engine::{self, Report},
    i18n::Message,
    jmap::Remote,
    scanner::LocalScanner,
    store::Store,
};
use anyhow::Result;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

struct FolderState {
    config: Config,
    scanner: Option<LocalScanner>,
    store: Option<Store>,
    due: Instant,
    backoff: u64,
    failed: bool,
    deferred: usize,
    blocked: usize,
}

pub struct SyncGroup {
    folders: Vec<FolderState>,
}

impl SyncGroup {
    pub fn new(config: &Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            folders: config
                .folder_configs()?
                .into_iter()
                .map(|config| FolderState {
                    scanner: None,
                    config,
                    store: None,
                    due: Instant::now(),
                    backoff: 5,
                    failed: false,
                    deferred: 0,
                    blocked: 0,
                })
                .collect(),
        })
    }

    pub fn invalidate(&mut self) {
        for folder in &mut self.folders {
            if let Some(scanner) = &mut folder.scanner {
                scanner.invalidate();
            }
            folder.due = Instant::now();
        }
    }

    pub fn ready(&self) -> bool {
        self.folders.iter().any(|folder| {
            Instant::now() >= folder.due
                || (!folder.failed && folder.scanner.as_ref().is_some_and(LocalScanner::ready))
        })
    }

    pub fn poll<R: Remote>(
        &mut self,
        remote: &mut R,
        cancel: &AtomicBool,
        log: &mut dyn FnMut(Message),
    ) -> Result<Report> {
        let mut total = Report::default();
        for folder in &mut self.folders {
            crate::runtime::check()?;
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Synchronization paused");
            }
            if Instant::now() < folder.due
                && (folder.failed || !folder.scanner.as_ref().is_some_and(LocalScanner::ready))
            {
                continue;
            }
            let result = (|| {
                // Never recreate a missing selected root or its private state directory.
                folder.config.validate()?;
                if folder.store.is_none() {
                    folder.store = Some(Store::open(&folder.config)?);
                }
                let prefix = folder.config.remote_folder_path()?;
                // Recursive watcher registration can enumerate directories on Linux.
                // Defer it until the power/cancellation gates and root checks have passed.
                let scanner = folder.scanner.get_or_insert_with(|| {
                    LocalScanner::new(&folder.config.local_folder, folder.config.full_scan_seconds)
                });
                engine::sync_with_scanner(
                    &folder.config,
                    folder.store.as_mut().unwrap(),
                    remote,
                    cancel,
                    &mut |mut message| {
                        message.prefix_paths(&prefix);
                        log(message);
                    },
                    scanner,
                )
            })();
            match result {
                Ok(report) => {
                    folder.failed = false;
                    folder.backoff = 5;
                    folder.deferred = report.deferred;
                    folder.blocked = report.blocked;
                    let delay = if report.needs_rescan {
                        0
                    } else if report.deferred > 0 {
                        3
                    } else {
                        folder.config.interval_seconds
                    };
                    folder.due = Instant::now() + Duration::from_secs(delay);
                    total.uploaded += report.uploaded;
                    total.downloaded += report.downloaded;
                    total.conflicts += report.conflicts;
                    total.recycled += report.recycled;
                    total.renamed += report.renamed;
                    total.needs_rescan |= report.needs_rescan;
                }
                Err(error) => {
                    crate::runtime::check()?;
                    if cancel.load(Ordering::Relaxed)
                        || error.downcast_ref::<auth::SignInRequired>().is_some()
                    {
                        return Err(error);
                    }
                    folder.failed = true;
                    folder.due = Instant::now() + Duration::from_secs(folder.backoff);
                    folder.backoff = (folder.backoff * 2).min(300);
                    if let Some(scanner) = &mut folder.scanner {
                        scanner.invalidate();
                    }
                    log(Message::new("log.folder_error")
                        .with("folder", folder.config.local_folder.display())
                        .with("error", format!("{error:#}")));
                }
            }
        }
        total.failed_folders = self.folders.iter().filter(|folder| folder.failed).count();
        total.deferred = self.folders.iter().map(|folder| folder.deferred).sum();
        total.blocked = self.folders.iter().map(|folder| folder.blocked).sum();
        Ok(total)
    }
}
