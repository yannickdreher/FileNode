//! Per-user Linux FUSE provider. Persistent metadata never implies local contents.
use super::{Identity, Kind, LocalEntry, Node};
use crate::{config::Config, jmap::Downloader};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
mod catalog;
mod control;
mod fuse;
use catalog::{Catalog, Entry, CHUNK};

type Registry = Mutex<HashMap<PathBuf, Arc<Mount>>>;
static ROOTS: OnceLock<Registry> = OnceLock::new();
fn roots() -> &'static Registry {
    ROOTS.get_or_init(Mutex::default)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}
fn locate(path: &Path) -> Result<Option<(Arc<State>, String)>> {
    let roots = roots().lock().unwrap_or_else(|e| e.into_inner());
    for (root, mount) in roots.iter() {
        if let Ok(relative) = path.strip_prefix(root) {
            return Ok(Some((
                mount.state.clone(),
                relative
                    .to_str()
                    .context("Non-UTF-8 cloud path")?
                    .to_owned(),
            )));
        }
    }
    Ok(None)
}
pub fn committed_entry(path: &Path) -> Result<Option<LocalEntry>> {
    let Some((state, relative)) = locate(path)? else {
        return Ok(None);
    };
    let db = state.catalog.lock().unwrap_or_else(|e| e.into_inner());
    Ok(db
        .path(&relative)?
        .filter(|e| !e.dirty)
        .and_then(|e| e.identity.map(|i| i.local)))
}
pub fn is_owned(path: &Path) -> Result<bool> {
    Ok(locate(path)?.is_some())
}
pub fn matches_node(path: &Path, node: &Node, local: &LocalEntry) -> Result<bool> {
    let Some((state, relative)) = locate(path)? else {
        return Ok(false);
    };
    let db = state.catalog.lock().unwrap_or_else(|e| e.into_inner());
    Ok(db.path(&relative)?.is_some_and(|e| {
        !e.dirty
            && e.identity.is_some_and(|i| {
                i.local == *local && i.node.id == node.id && i.node.blob_id == node.blob_id
            })
    }))
}
pub fn is_registered(config: &Config) -> Result<bool> {
    if locate(&config.local_folder)?.is_some() {
        return Ok(true);
    }
    let directory = config.private_directory()?.join("linux-cloud");
    Ok(directory.join("namespace.sqlite").exists() && !directory.join("materialized").exists())
}

pub use control::command;

pub fn validate_root(config: &Config) -> Result<()> {
    let root = config.local_folder.canonicalize()?;
    if let Some((state, relative)) = locate(&root)? {
        if relative.is_empty() && state.key == super::root_key(config)? {
            return Ok(());
        }
        bail!("Linux cloud folders must not overlap or change account while mounted");
    }
    if fs::symlink_metadata(&config.local_folder)?
        .file_type()
        .is_symlink()
    {
        bail!("Cloud mount must not be a symbolic link");
    }
    if fs::read_dir(&root)?.next().is_some() {
        bail!("Linux Files On-Demand needs an empty dedicated mount folder. Existing files will not be hidden or moved.");
    }
    if !Path::new("/dev/fuse").exists() {
        bail!("Linux Files On-Demand requires FUSE 3 (/dev/fuse and fusermount3)");
    }
    for line in fs::read_to_string("/proc/self/mountinfo")?.lines() {
        let Some((fields, filesystem)) = line.split_once(" - ") else {
            continue;
        };
        let filesystem = filesystem.split_whitespace().next().unwrap_or("");
        if filesystem != "fuse" && !filesystem.starts_with("fuse.") {
            continue;
        }
        if let Some(encoded) = fields.split_whitespace().nth(4) {
            let mount = PathBuf::from(
                encoded
                    .replace("\\040", " ")
                    .replace("\\011", "\t")
                    .replace("\\012", "\n")
                    .replace("\\134", "\\"),
            );
            if root.starts_with(&mount) || mount.starts_with(&root) {
                bail!(
                    "Linux cloud folder overlaps an existing FUSE mount: {}",
                    mount.display()
                );
            }
        }
    }
    Ok(())
}

struct State {
    root: PathBuf,
    cache: PathBuf,
    key: String,
    catalog: Mutex<Catalog>,
    io: Mutex<()>,
    file_locks: Mutex<HashMap<u64, Arc<Mutex<()>>>>,
    downloader: RwLock<Arc<dyn Downloader>>,
    stopping: AtomicBool,
    detaching: AtomicBool,
    opens: Mutex<HashMap<u64, usize>>,
    activity: crate::activity::Captured,
    tasks: Mutex<Option<std::sync::mpsc::SyncSender<fuse::IoRequest>>>,
    publishing: Mutex<HashMap<String, (u32, Identity)>>,
}
impl State {
    fn file_lock(&self, ino: u64) -> Arc<Mutex<()>> {
        self.file_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(ino)
            .or_default()
            .clone()
    }
    fn entry(&self, ino: u64) -> Result<Entry> {
        self.catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(ino)?
            .context("Cloud inode is missing")
    }
    fn file(&self, ino: u64) -> Result<File> {
        Ok(OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.cache.join(ino.to_string()))?)
    }
    fn hydrate(&self, ino: u64, offset: u64, length: u64) -> Result<()> {
        let entry = self.entry(ino)?;
        if entry.dirty || entry.kind == Kind::Directory || length == 0 {
            return Ok(());
        }
        let identity = entry
            .identity
            .as_ref()
            .context("Cloud file has no committed server version")?;
        let end = offset.saturating_add(length).min(entry.size);
        let mut file = self.file(ino)?;
        let downloader = self
            .downloader
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for chunk in offset / CHUNK..end.div_ceil(CHUNK) {
            if entry.chunks.contains(&chunk) {
                continue;
            }
            if self.stopping.load(Ordering::SeqCst) {
                bail!("Cloud provider is stopping")
            }
            let start = chunk * CHUNK;
            let length = CHUNK.min(entry.size - start);
            let _scope = self.activity.enter();
            let mut activity = crate::activity::FileActivity::begin(
                entry.path.as_deref().unwrap_or(""),
                "download",
                length,
            );
            let mut bytes = Vec::with_capacity(length as usize);
            downloader.read_range(&identity.node, start, length, &self.stopping, &mut |part| {
                if bytes.len() as u64 + part.len() as u64 > length {
                    bail!("Cloud range exceeds expected length")
                }
                bytes.extend_from_slice(part);
                Ok(())
            })?;
            if bytes.len() as u64 != length {
                bail!("Cloud range ended before its advertised length")
            }
            file.seek(SeekFrom::Start(start))?;
            file.write_all(&bytes)?;
            file.sync_data()?;
            let db = self.catalog.lock().unwrap_or_else(|e| e.into_inner());
            let mut current = db.entry(ino)?.context("Cloud file disappeared")?;
            current.chunks.insert(chunk);
            db.save(&current)?;
            activity.finish("synced");
        }
        Ok(())
    }
    fn read(&self, ino: u64, offset: u64, size: u32) -> Result<Vec<u8>> {
        let lock = self.file_lock(ino);
        let _io = lock.lock().unwrap_or_else(|e| e.into_inner());
        let entry = self.entry(ino)?;
        let count = (size as u64).min(entry.size.saturating_sub(offset));
        self.hydrate(ino, offset, count)?;
        let mut data = vec![0; count as usize];
        let mut file = self.file(ino)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut data)?;
        Ok(data)
    }
    fn write(&self, ino: u64, offset: u64, data: &[u8], truncate: Option<u64>) -> Result<()> {
        let lock = self.file_lock(ino);
        let _io = lock.lock().unwrap_or_else(|e| e.into_inner());
        self.check_mutation()?;
        let entry = self.entry(ino)?;
        if truncate != Some(0) {
            self.hydrate(ino, 0, entry.size)?;
        }
        // Persist dirty *before* touching bytes: a crash cannot leave edited data
        // advertised as the committed remote version or eligible for eviction.
        {
            let db = self.catalog.lock().unwrap_or_else(|e| e.into_inner());
            let mut entry = db.entry(ino)?.context("Missing cloud inode")?;
            entry.dirty = true;
            entry.modified = now();
            db.save(&entry)?;
        }
        let mut file = self.file(ino)?;
        if let Some(size) = truncate {
            file.set_len(size)?;
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;
        file.sync_all()?;
        let db = self.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let mut entry = db.entry(ino)?.context("Missing cloud inode")?;
        entry.size = file.metadata()?.len();
        entry.chunks.clear();
        db.save(&entry)?;
        Ok(())
    }
    fn availability(&self, path: &str, pin: bool) -> Result<()> {
        self.check_mutation()?;
        let entries = self
            .catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries()?;
        let selected: Vec<_> = entries
            .into_iter()
            .filter(|e| {
                e.path.as_ref().is_some_and(|p| {
                    path.is_empty() || p == path || p.starts_with(&format!("{path}/"))
                })
            })
            .collect();
        if !pin
            && selected.iter().any(|e| {
                e.dirty
                    || self
                        .opens
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(&e.ino)
                        .copied()
                        .unwrap_or(0)
                        != 0
            })
        {
            bail!("Unsynchronized or open files cannot be evicted");
        }
        for selected in selected {
            // Network reads serialize only the same inode. A slow download must
            // not hold the namespace gate or block cached reads of other files.
            let namespace = if pin {
                None
            } else {
                Some(self.io.lock().unwrap_or_else(|e| e.into_inner()))
            };
            let lock = self.file_lock(selected.ino);
            let _file = lock.lock().unwrap_or_else(|e| e.into_inner());
            self.check_mutation()?;
            let mut entry = {
                let db = self.catalog.lock().unwrap_or_else(|e| e.into_inner());
                let mut entry = db.entry(selected.ino)?.context("Missing cloud inode")?;
                if !pin
                    && (entry.dirty
                        || self
                            .opens
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&entry.ino)
                            .copied()
                            .unwrap_or(0)
                            > 0)
                {
                    bail!("Unsynchronized or open files cannot be evicted");
                }
                entry.pinned = pin;
                db.save(&entry)?;
                entry
            };
            if entry.kind == Kind::Directory {
                continue;
            }
            if pin {
                self.hydrate(entry.ino, 0, entry.size)?;
            } else {
                // Forget valid ranges durably before releasing their cache space.
                entry.chunks.clear();
                self.catalog
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .save(&entry)?;
                self.file(entry.ino)?.set_len(0)?;
            }
            drop(namespace);
        }
        Ok(())
    }
    fn check_mutation(&self) -> Result<()> {
        if self.detaching.load(Ordering::SeqCst) {
            return Err(std::io::Error::from_raw_os_error(libc::EBUSY).into());
        }
        Ok(())
    }
}

struct Mount {
    state: Arc<State>,
    session: Mutex<Option<fuser::BackgroundSession>>,
    control: Mutex<Option<control::Server>>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}
impl Mount {
    fn shutdown(&self) -> Result<()> {
        self.state.stopping.store(true, Ordering::SeqCst);
        self.control
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        self.state
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            let _ = worker.join();
        }
        if let Some(session) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            session.umount_and_join()?;
        }
        Ok(())
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
pub struct Provider {
    state: Arc<State>,
    _mount: Arc<Mount>,
}
impl Provider {
    pub fn connect(config: &Config, downloader: Arc<dyn Downloader>) -> Result<Self> {
        validate_root(config)?;
        let root = config.local_folder.canonicalize()?;
        let key = super::root_key(config)?;
        if let Some(mount) = roots()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&root)
            .cloned()
        {
            if mount.state.key != key {
                bail!("Cloud mount belongs to another profile");
            }
            let provider = Self {
                state: mount.state.clone(),
                _mount: mount,
            };
            provider.set_downloader(downloader);
            return Ok(provider);
        }
        let directory = config.private_directory()?.join("linux-cloud");
        crate::store::ensure_state_directory(&directory)?;
        let cache = directory.join("contents");
        crate::store::ensure_state_directory(&cache)?;
        let catalog = Catalog::open(&directory.join("namespace.sqlite"), &key)?;
        // Dirty is persisted before each write. Recover the actual durable size
        // if the process exited after writing but before its metadata commit.
        for mut entry in catalog
            .entries()?
            .into_iter()
            .filter(|e| e.dirty && e.kind == Kind::File)
        {
            entry.size = fs::metadata(cache.join(entry.ino.to_string()))
                .context("Recover dirty cloud file")?
                .len();
            catalog.save(&entry)?;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel::<fuse::IoRequest>(32);
        let state = Arc::new(State {
            root: root.clone(),
            cache,
            key: key.clone(),
            catalog: Mutex::new(catalog),
            io: Mutex::new(()),
            file_locks: Mutex::new(HashMap::new()),
            downloader: RwLock::new(downloader),
            stopping: AtomicBool::new(false),
            detaching: AtomicBool::new(false),
            opens: Mutex::new(HashMap::new()),
            activity: crate::activity::Scope::capture(),
            tasks: Mutex::new(Some(sender)),
            publishing: Mutex::new(HashMap::new()),
        });
        // Network I/O has a bounded worker queue separate from FUSE dispatch.
        // Even slow downloads must leave metadata/status requests serviceable.
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let receiver = receiver.clone();
            workers.push(std::thread::spawn(move || loop {
                let request = receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                match request {
                    Ok(request) => request.run(),
                    Err(_) => break,
                }
            }));
        }
        let mut options = fuser::Config::default();
        options.mount_options = vec![
            fuser::MountOption::FSName("FileNode".into()),
            fuser::MountOption::Subtype("filenode".into()),
            fuser::MountOption::DefaultPermissions,
            fuser::MountOption::NoDev,
            fuser::MountOption::NoSuid,
        ];
        options.n_threads = Some(4);
        let session = fuser::spawn_mount(fuse::Filesystem(state.clone()), &root, &options)
            .context("Mount Linux Files On-Demand (install fuse3 and allow /dev/fuse)")?;
        let mount = Arc::new(Mount {
            state: state.clone(),
            session: Mutex::new(Some(session)),
            control: Mutex::new(None),
            workers: Mutex::new(workers),
        });
        *mount.control.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(control::Server::start(&state)?);
        let marker = directory.join("materialized");
        if marker.exists() {
            fs::remove_file(marker)?;
        }
        roots()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(root, mount.clone());
        Ok(Self {
            state,
            _mount: mount,
        })
    }
    pub fn set_downloader(&self, downloader: Arc<dyn Downloader>) {
        *self
            .state
            .downloader
            .write()
            .unwrap_or_else(|e| e.into_inner()) = downloader;
    }
    pub fn publish(
        &self,
        relative: &str,
        node: &Node,
        local: Option<&LocalEntry>,
    ) -> Result<LocalEntry> {
        let target = crate::filesystem::checked_path(&self.state.root, relative)?;
        let identity = Identity::new(&self.state.key, node, local);
        if self
            .state
            .catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .path(relative)?
            .is_none()
        {
            // Go through the VFS so native inotify/GIO/KIO directory monitors
            // receive remote creates too. Only this originating thread may
            // consume the prepared remote identity in the FUSE create callback.
            let origin = unsafe { libc::syscall(libc::SYS_gettid) as u32 };
            self.state
                .publishing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(relative.into(), (origin, identity.clone()));
            let result = if node.kind == Kind::Directory {
                fs::create_dir(&target)
            } else {
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&target)
                    .map(drop)
            };
            self.state
                .publishing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(relative);
            result.context("Publish remote cloud entry through native filesystem notifications")?;
            return Ok(identity.local);
        }
        let _io = self.state.io.lock().unwrap_or_else(|e| e.into_inner());
        let existing = self
            .state
            .catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .path(relative)?;
        let lock = existing.as_ref().map(|e| self.state.file_lock(e.ino));
        let _file = lock
            .as_ref()
            .map(|lock| {
                lock.try_lock()
                    .map_err(|_| anyhow::anyhow!("Cloud file is busy; remote commit deferred"))
            })
            .transpose()?;
        let db = self.state.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let existing = db.path(relative)?;
        let mut entry = match existing {
            Some(e) => e,
            None => bail!("Cloud entry changed before publication; rescan required"),
        };
        if entry.kind != node.kind {
            bail!("Cloud file type changed; rescan required")
        }
        if entry.dirty {
            let Some(local) = local else {
                bail!("Cannot replace unsynchronized cloud data")
            };
            if local.kind == Kind::File {
                let actual = crate::filesystem::hash_file(
                    &self.state.cache.join(entry.ino.to_string()),
                    u64::MAX,
                )?;
                if actual != *local {
                    bail!("Local cloud file changed before commit")
                }
            }
        }
        if !entry.dirty
            && entry
                .identity
                .as_ref()
                .is_some_and(|old| old.node.blob_id != node.blob_id)
        {
            if self
                .state
                .opens
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&entry.ino)
                .copied()
                .unwrap_or(0)
                > 0
            {
                bail!("Cloud file is open; remote update deferred")
            }
            entry.chunks.clear();
            db.save(&entry)?;
            self.state.file(entry.ino)?.set_len(0)?;
        }
        if entry.dirty {
            entry.chunks = (0..node.size.div_ceil(CHUNK)).collect();
        }
        entry.identity = Some(identity.clone());
        entry.size = node.size;
        entry.dirty = false;
        db.save(&entry)?;
        drop(db);
        drop(_file);
        drop(_io);
        // Notify native file monitors of remote metadata/content changes without
        // reading the data stream. Failure does not undo the committed identity.
        if let Ok(file) = File::open(target) {
            let _ = file.set_modified(SystemTime::now());
        }
        Ok(identity.local)
    }
    pub fn availability(&self) -> Result<()> {
        let entries = self
            .state
            .catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries()?;
        for entry in entries.into_iter().filter(|e| e.pinned && !e.complete()) {
            self.state
                .availability(entry.path.as_deref().unwrap_or(""), true)?;
        }
        Ok(())
    }
}
pub fn shutdown() {
    let mounts: Vec<_> = roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain()
        .map(|(_, m)| m)
        .collect();
    for mount in mounts {
        if let Err(error) = mount.shutdown() {
            crate::logger::event(
                crate::logger::Level::Error,
                399,
                "cloud_unmount_error",
                Some(&mount.state.root.to_string_lossy()),
                &format!("{error:#}"),
            );
        }
    }
}

pub fn removed_roots(before: &Config, after: &Config) -> Result<Vec<Config>> {
    let keep: Vec<_> = after
        .folder_configs()?
        .into_iter()
        .filter(|c| c.files_on_demand)
        .map(|c| super::root_key(&c))
        .collect::<Result<_>>()?;
    for root in after.folder_configs()?.iter().filter(|c| c.files_on_demand) {
        validate_root(root)?;
    }
    let removed: Vec<_> = before
        .folder_configs()?
        .into_iter()
        .filter(|c| c.files_on_demand)
        .filter_map(|c| match super::root_key(&c) {
            Ok(k) if keep.contains(&k) => None,
            _ => Some(c),
        })
        .collect();
    for config in &removed {
        check_detach(config)?;
    }
    Ok(removed)
}

fn detach_entries(config: &Config) -> Result<Vec<Entry>> {
    if let Some((state, _)) = locate(&config.local_folder)? {
        if state
            .opens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|v| *v > 0)
        {
            bail!("Close open files before stopping cloud sync");
        }
        return state
            .catalog
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries();
    }
    Catalog::open(
        &config
            .private_directory()?
            .join("linux-cloud/namespace.sqlite"),
        &super::root_key(config)?,
    )?
    .entries()
}
fn check_detach(config: &Config) -> Result<()> {
    if detach_entries(config)?.iter().any(|e| !e.complete()) {
        bail!("Choose Always keep on this device and wait for every download before stopping cloud sync");
    }
    Ok(())
}
pub fn detach(config: &Config) -> Result<()> {
    let root = config.local_folder.canonicalize()?;
    let directory = config.private_directory()?.join("linux-cloud");
    let active = roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&root)
        .cloned();
    let entries = if let Some(mount) = &active {
        let _io = mount.state.io.lock().unwrap_or_else(|e| e.into_inner());
        check_detach(config)?;
        let entries = detach_entries(config)?;
        mount.state.detaching.store(true, Ordering::SeqCst);
        entries
    } else {
        check_detach(config)?;
        detach_entries(config)?
    };
    let mount = roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&root);
    if let Some(mount) = mount {
        mount.shutdown()?;
    }
    for entry in entries.iter().filter(|e| e.ino != 1) {
        let relative = entry.path.as_deref().context("Unlinked cloud file")?;
        let target = crate::filesystem::checked_path(&root, relative)?;
        if entry.kind == Kind::Directory {
            fs::create_dir_all(target)?;
        } else {
            fs::create_dir_all(target.parent().context("Materialized parent")?)?;
            let source = directory.join("contents").join(entry.ino.to_string());
            if target.exists() {
                if crate::filesystem::hash_file(&source, u64::MAX)?
                    != crate::filesystem::hash_file(&target, u64::MAX)?
                {
                    bail!("Materialization target contains different data");
                }
            } else {
                let mut staged = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
                std::io::copy(&mut File::open(&source)?, &mut staged)?;
                staged.as_file().sync_all()?;
                let copy = crate::filesystem::hash_file(staged.path(), u64::MAX)?;
                if copy.size != entry.size
                    || copy != crate::filesystem::hash_file(&source, u64::MAX)?
                {
                    bail!("Materialized cloud file failed verification");
                }
                staged.persist_noclobber(&target)?;
            }
        }
    }
    // Keep both cache and namespace for recovery. Ordinary files now use their
    // actual hashes instead of the virtual comparison identity.
    let store = crate::store::Store::open(config)?;
    for (path, mut baseline) in store.baselines()? {
        if entries.iter().any(|e| {
            e.path.as_deref() == Some(&path)
                && !e.dirty
                && e.identity
                    .as_ref()
                    .is_some_and(|i| i.local == baseline.local)
        }) {
            baseline.local = if baseline.local.kind == Kind::Directory {
                baseline.local
            } else {
                crate::filesystem::hash_file(&root.join(&path), u64::MAX)?
            };
            store.db.execute(
                "UPDATE baseline SET value=?1 WHERE path=?2",
                rusqlite::params![serde_json::to_string(&baseline)?, path],
            )?;
        }
    }
    File::create(directory.join("materialized"))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
