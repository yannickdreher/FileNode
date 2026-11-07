//! Event-invalidated hash cache with periodic full verification and conservative fallback.
//! The complete namespace is still enumerated before planning; a watcher is only a hint.
use crate::{
    filesystem,
    model::{LocalEntry, LocalTree},
};
use anyhow::{bail, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    length: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

fn fingerprint(path: &Path) -> Result<Fingerprint> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("File type changed while scanning");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            bail!("Reparse point encountered while scanning");
        }
    }
    Ok(Fingerprint {
        length: metadata.len(),
        modified: metadata.modified()?,
        created: metadata.created().ok(),
        #[cfg(unix)]
        identity: {
            use std::os::unix::fs::MetadataExt;
            (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
        },
    })
}

#[derive(Default)]
struct Pending {
    paths: HashSet<String>,
    full: bool,
    last_event: Option<Instant>,
    first_event: Option<Instant>,
}

#[derive(Default)]
struct Signals {
    pending: Mutex<Pending>,
    wake: Condvar,
    failed: AtomicBool,
}

impl Signals {
    fn event(&self, root: &Path, event: notify::Result<Event>) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
            Ok(event) => {
                if event.need_rescan() || event.paths.is_empty() {
                    pending.full = true;
                }
                let mut relevant = pending.full;
                for path in event.paths {
                    match path.strip_prefix(root).ok().and_then(Path::to_str) {
                        Some(relative) => {
                            let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
                            if filesystem::ignored(&relative) {
                                continue;
                            }
                            relevant = true;
                            if relative.is_empty() {
                                pending.full = true;
                                self.failed.store(true, Ordering::Relaxed);
                            }
                            if pending.paths.len() < 100_000 {
                                pending.paths.insert(relative);
                            } else {
                                pending.full = true;
                            }
                        }
                        None => {
                            pending.full = true;
                            relevant = true;
                        }
                    }
                }
                if !relevant {
                    return;
                }
            }
            Err(_) => {
                pending.full = true;
                self.failed.store(true, Ordering::Relaxed);
            }
        }
        let now = Instant::now();
        pending.last_event = Some(now);
        pending.first_event.get_or_insert(now);
        self.wake.notify_all();
    }
}

pub struct LocalScanner {
    root: PathBuf,
    cache: HashMap<String, (Fingerprint, LocalEntry)>,
    signals: Arc<Signals>,
    _watcher: Option<RecommendedWatcher>,
    last_full: Option<Instant>,
    interval: Duration,
    hashed_files: usize,
    observed: HashMap<String, (Fingerprint, Instant)>,
    deferred: HashSet<String>,
    settle: Duration,
}

impl LocalScanner {
    pub fn new(root: &Path, full_scan_seconds: u64) -> Self {
        let root = root.to_path_buf();
        let signals = Arc::new(Signals::default());
        let watched_root = root.clone();
        let events = signals.clone();
        let watcher = notify::recommended_watcher(move |event| events.event(&watched_root, event))
            .and_then(|mut watcher| {
                watcher.watch(&root, RecursiveMode::Recursive)?;
                Ok(watcher)
            })
            .ok();
        Self {
            root,
            cache: HashMap::new(),
            signals,
            _watcher: watcher,
            last_full: None,
            interval: Duration::from_secs(full_scan_seconds),
            hashed_files: 0,
            observed: HashMap::new(),
            deferred: HashSet::new(),
            settle: Duration::ZERO,
        }
    }

    pub fn set_settle_seconds(&mut self, seconds: u64) {
        self.settle = Duration::from_secs(seconds);
    }
    pub fn deferred_roots(&self) -> Vec<String> {
        // An unresolved child also protects its ancestors from being deleted or moved.
        self.deferred
            .iter()
            .map(|path| path.split('/').next().unwrap_or(path).to_owned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn watching(&self) -> bool {
        self._watcher.is_some() && !self.signals.failed.load(Ordering::Relaxed)
    }
    pub fn hashed_files(&self) -> usize {
        self.hashed_files
    }

    /// Explicit checks, reconnects and watcher overflows invalidate cached hashes.
    pub fn invalidate(&mut self) {
        self.last_full = None;
    }

    pub fn scan(&mut self, max_bytes: u64) -> Result<LocalTree> {
        let pending = {
            let mut pending = self
                .signals
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *pending)
        };
        let full = !self.watching()
            || pending.full
            || self.last_full.is_none_or(|t| t.elapsed() >= self.interval);
        if full {
            self.cache.clear();
        } else {
            self.cache.retain(|path, _| {
                !pending
                    .paths
                    .iter()
                    .any(|dirty| path == dirty || path.starts_with(&format!("{dirty}/")))
            });
        }
        self.hashed_files = 0;
        self.deferred.clear();
        let result = filesystem::scan_with(&self.root, |path, relative| {
            let before = fingerprint(path)?;
            if before.length > max_bytes {
                bail!("File exceeds size limit: {relative}");
            }
            if let Some((old, entry)) = self.cache.get(relative) {
                if old == &before {
                    return Ok(entry.clone());
                }
            }
            let observed = self
                .observed
                .entry(relative.to_owned())
                .or_insert((before.clone(), Instant::now()));
            if observed.0 != before {
                *observed = (before.clone(), Instant::now());
            }
            let unsettled = observed.1.elapsed() < self.settle;
            let entry = if unsettled {
                None
            } else {
                match filesystem::hash_file(path, max_bytes) {
                    Ok(entry) => Some(entry),
                    Err(error) => {
                        crate::runtime::check()?;
                        let _ = error;
                        None
                    }
                }
            };
            let Some(entry) = entry else {
                self.deferred.insert(relative.to_owned());
                return Ok(self
                    .cache
                    .get(relative)
                    .map(|(_, entry)| entry.clone())
                    .unwrap_or(LocalEntry {
                        kind: crate::model::Kind::File,
                        hash: "deferred".into(),
                        size: before.length,
                    }));
            };
            if fingerprint(path)? != before {
                bail!("File changed during scanning: {relative}");
            }
            self.hashed_files += 1;
            self.cache
                .insert(relative.to_owned(), (before, entry.clone()));
            Ok(entry)
        });
        match result {
            Ok(tree) => {
                self.cache.retain(|path, _| tree.contains_key(path));
                self.observed.retain(|path, _| tree.contains_key(path));
                if full {
                    self.last_full = Some(Instant::now());
                }
                Ok(tree)
            }
            Err(error) => {
                self.invalidate();
                Err(error)
            }
        }
    }

    /// Wait for remote polling or a debounced local burst. Notifications never perform I/O.
    /// A 2-second cap prevents continuous writes from starving reconciliation.
    pub fn wait(&self, maximum: Duration) {
        let maximum = maximum.min(
            self.last_full
                .map(|time| self.interval.saturating_sub(time.elapsed()))
                .unwrap_or(maximum),
        );
        let pending = self
            .signals
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let delay = if let Some(last) = pending.last_event {
            let quiet = Duration::from_millis(600).saturating_sub(last.elapsed());
            let cap = Duration::from_secs(2)
                .saturating_sub(pending.first_event.unwrap_or(last).elapsed());
            maximum.min(quiet.min(cap))
        } else {
            maximum
        };
        drop(self.signals.wake.wait_timeout(pending, delay));
    }

    pub fn ready(&self) -> bool {
        if self
            .last_full
            .is_some_and(|time| time.elapsed() >= self.interval)
        {
            return true;
        }
        let pending = self
            .signals
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending
            .last_event
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(600))
            || pending
                .first_event
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_reuses_hashes_and_events_invalidate_same_metadata_contents() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a.txt");
        std::fs::write(&file, "one").unwrap();
        let mut scanner = LocalScanner::new(root.path(), 900);
        let first = scanner.scan(100).unwrap();
        assert_eq!(scanner.hashed_files(), 1);
        if scanner.watching() {
            // Drain real notifications deterministically; inject the invalidation below.
            std::thread::sleep(Duration::from_millis(100));
            scanner.scan(100).unwrap();
            scanner.scan(100).unwrap();
            assert_eq!(scanner.hashed_files(), 0);
        }
        let modified = file.metadata().unwrap().modified().unwrap();
        std::fs::write(&file, "two").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        scanner
            .signals
            .event(root.path(), Ok(Event::new(EventKind::Any).add_path(file)));
        let updated = scanner.scan(100).unwrap();
        assert_ne!(first["a.txt"].hash, updated["a.txt"].hash);
        assert_eq!(scanner.hashed_files(), 1);
        scanner.invalidate();
        scanner.scan(100).unwrap();
        assert_eq!(scanner.hashed_files(), 1);
    }

    #[test]
    fn errors_and_overflow_force_full_verification_and_private_events_do_not_wake() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), "one").unwrap();
        let mut scanner = LocalScanner::new(root.path(), 900);
        scanner.scan(100).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        scanner.scan(100).unwrap();
        scanner.signals.event(
            root.path(),
            Ok(Event::new(EventKind::Any)
                .add_path(root.path().join(".noritec-drive/state.sqlite"))),
        );
        assert!(scanner.signals.pending.lock().unwrap().last_event.is_none());
        scanner
            .signals
            .event(root.path(), Err(notify::Error::generic("lost events")));
        scanner.scan(100).unwrap();
        assert_eq!(scanner.hashed_files(), 1);
        let event = Event::new(EventKind::Any).set_flag(notify::event::Flag::Rescan);
        scanner.signals.event(root.path(), Ok(event));
        scanner.scan(100).unwrap();
        assert_eq!(scanner.hashed_files(), 1);
    }
}
