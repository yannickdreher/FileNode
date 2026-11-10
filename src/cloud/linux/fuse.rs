use super::*;
use fuser::*;
use std::{ffi::OsStr, time::Duration};

pub(super) struct Filesystem(pub Arc<State>);
pub(super) enum IoRequest {
    Read {
        state: Arc<State>,
        ino: u64,
        offset: u64,
        size: u32,
        reply: ReplyData,
    },
    Write {
        state: Arc<State>,
        ino: u64,
        offset: u64,
        data: Vec<u8>,
        reply: ReplyWrite,
    },
}
impl IoRequest {
    pub fn run(self) {
        match self {
            Self::Read {
                state,
                ino,
                offset,
                size,
                reply,
            } => match state.read(ino, offset, size) {
                Ok(data) => reply.data(&data),
                Err(e) => reply.error(error(e)),
            },
            Self::Write {
                state,
                ino,
                offset,
                data,
                reply,
            } => match state.write(ino, offset, &data, None) {
                Ok(()) => reply.written(data.len() as u32),
                Err(e) => reply.error(error(e)),
            },
        }
    }
    fn reject(self) {
        match self {
            Self::Read { reply, .. } => reply.error(Errno::EAGAIN),
            Self::Write { reply, .. } => reply.error(Errno::EAGAIN),
        }
    }
}
impl State {
    fn queue(&self, request: IoRequest) {
        use std::sync::mpsc::TrySendError;
        let sender = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = sender.as_ref() {
            if let Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) =
                sender.try_send(request)
            {
                job.reject();
            }
        } else {
            request.reject();
        }
    }
}
const TTL: Duration = Duration::ZERO;
fn error(error: anyhow::Error) -> Errno {
    let code = error
        .chain()
        .find_map(|e| {
            e.downcast_ref::<std::io::Error>()
                .and_then(|e| e.raw_os_error())
        })
        .unwrap_or(libc::EIO);
    Errno::from_i32(code)
}
fn os_error(code: i32) -> anyhow::Error {
    std::io::Error::from_raw_os_error(code).into()
}
fn path(db: &Catalog, parent: INodeNo, name: &OsStr) -> Result<String> {
    let parent = db.entry(parent.0)?.ok_or_else(|| os_error(libc::ENOENT))?;
    if parent.kind != Kind::Directory {
        return Err(os_error(libc::ENOTDIR));
    }
    let parent = parent.path.context("Unlinked parent")?;
    let name = name.to_str().ok_or_else(|| os_error(libc::EINVAL))?;
    crate::filesystem::validate_name(name).map_err(|_| os_error(libc::EINVAL))?;
    let result = if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    };
    if result.len() > 240 {
        return Err(os_error(libc::ENAMETOOLONG));
    }
    Ok(result)
}
fn kind(entry: &Entry) -> FileType {
    if entry.kind == Kind::Directory {
        FileType::Directory
    } else {
        FileType::RegularFile
    }
}
fn attr(entry: &Entry) -> FileAttr {
    let time = UNIX_EPOCH + Duration::from_nanos(entry.modified);
    FileAttr {
        ino: INodeNo(entry.ino),
        size: entry.size,
        blocks: if entry.dirty {
            entry.size.div_ceil(512)
        } else {
            (entry.chunks.len() as u64 * CHUNK)
                .min(entry.size)
                .div_ceil(512)
        },
        atime: time,
        mtime: time,
        ctime: time,
        crtime: time,
        kind: kind(entry),
        perm: entry.mode,
        nlink: if entry.path.is_none() {
            0
        } else if entry.kind == Kind::Directory {
            2
        } else {
            1
        },
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}
impl Filesystem {
    fn create_entry(
        &self,
        parent: INodeNo,
        name: &OsStr,
        kind: Kind,
        mode: u32,
        umask: u32,
        origin: u32,
    ) -> Result<Entry> {
        let _io = self.0.io.lock().unwrap_or_else(|e| e.into_inner());
        self.0.check_mutation()?;
        let db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let path = path(&db, parent, name)?;
        if db.path(&path)?.is_some() {
            return Err(os_error(libc::EEXIST));
        }
        let mut entry = db.allocate(&path, kind)?;
        entry.mode = (mode & !umask & 0o777) as u16;
        entry.pinned = db
            .path(path.rsplit_once('/').map(|p| p.0).unwrap_or(""))?
            .is_some_and(|p| p.pinned);
        let identity = {
            let mut publishing = self.0.publishing.lock().unwrap_or_else(|e| e.into_inner());
            if publishing.get(&path).is_some_and(|(pid, _)| *pid == origin) {
                publishing.remove(&path).map(|(_, identity)| identity)
            } else {
                None
            }
        };
        if let Some(identity) = identity {
            if identity.node.kind != kind {
                return Err(os_error(libc::EINVAL));
            }
            entry.size = identity.node.size;
            entry.identity = Some(identity);
            entry.dirty = false;
        }
        if kind == Kind::File {
            let file = self.0.file(entry.ino)?;
            file.set_len(0)?;
            file.sync_all()?;
        }
        db.save(&entry)?;
        Ok(entry)
    }
    fn remove(&self, parent: INodeNo, name: &OsStr, directory: bool) -> Result<()> {
        let _io = self.0.io.lock().unwrap_or_else(|e| e.into_inner());
        self.0.check_mutation()?;
        let db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let path = path(&db, parent, name)?;
        let mut entry = db.path(&path)?.ok_or_else(|| os_error(libc::ENOENT))?;
        if (entry.kind == Kind::Directory) != directory {
            return Err(os_error(if directory {
                libc::ENOTDIR
            } else {
                libc::EISDIR
            }));
        }
        if directory
            && db.entries()?.iter().any(|e| {
                e.path
                    .as_ref()
                    .is_some_and(|p| p.starts_with(&format!("{path}/")))
            })
        {
            return Err(os_error(libc::ENOTEMPTY));
        }
        // Keep cache contents for open descriptors and uncertain server commits.
        // Unlinking the namespace is enough for the ordinary three-way planner.
        entry.path = None;
        db.save(&entry)?;
        Ok(())
    }
    fn rename_entry(
        &self,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
    ) -> Result<()> {
        let _io = self.0.io.lock().unwrap_or_else(|e| e.into_inner());
        self.0.check_mutation()?;
        let mut db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let from = path(&db, parent, name)?;
        let to = path(&db, newparent, newname)?;
        if from == to {
            return Ok(());
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(os_error(libc::EINVAL));
        }
        if flags.bits() & !libc::RENAME_NOREPLACE != 0 {
            return Err(os_error(libc::EINVAL));
        }
        let source = db.path(&from)?.ok_or_else(|| os_error(libc::ENOENT))?;
        let target = db.path(&to)?;
        if flags.bits() & libc::RENAME_NOREPLACE != 0 && target.is_some() {
            return Err(os_error(libc::EEXIST));
        }
        let entries = db.entries()?;
        if let Some(target) = &target {
            if target.kind != source.kind {
                return Err(os_error(if target.kind == Kind::Directory {
                    libc::EISDIR
                } else {
                    libc::ENOTDIR
                }));
            }
            if target.kind == Kind::Directory
                && entries.iter().any(|e| {
                    e.path
                        .as_ref()
                        .is_some_and(|p| p.starts_with(&format!("{to}/")))
                })
            {
                return Err(os_error(libc::ENOTEMPTY));
            }
        }
        let tx = db.db.transaction()?;
        if let Some(mut target) = target {
            target.path = None;
            tx.execute(
                "UPDATE entries SET path=NULL,parent=NULL,value=?2 WHERE ino=?1",
                rusqlite::params![target.ino, serde_json::to_string(&target)?],
            )?;
        }
        for mut entry in entries.into_iter().filter(|e| {
            e.path
                .as_ref()
                .is_some_and(|p| p == &from || p.starts_with(&format!("{from}/")))
        }) {
            let old = entry.path.take().unwrap();
            entry.path = Some(format!("{to}{}", &old[from.len()..]));
            tx.execute(
                "UPDATE entries SET path=?2,value=?3,parent=?4 WHERE ino=?1",
                rusqlite::params![
                    entry.ino,
                    entry.path,
                    serde_json::to_string(&entry)?,
                    entry.parent()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
impl fuser::Filesystem for Filesystem {
    fn lookup(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let result = (|| {
            let db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
            db.path(&path(&db, parent, name)?)?
                .ok_or_else(|| os_error(libc::ENOENT))
        })();
        match result {
            Ok(e) => reply.entry(&TTL, &attr(&e), Generation(0)),
            Err(e) => reply.error(error(e)),
        }
    }
    fn getattr(&self, _: &Request, ino: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        match self.0.entry(ino.0) {
            Ok(e) => reply.attr(&TTL, &attr(&e)),
            Err(e) => reply.error(error(e)),
        }
    }
    fn setattr(
        &self,
        _: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _: Option<SystemTime>,
        _: Option<FileHandle>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<SystemTime>,
        _: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let result = (|| {
            if uid.is_some_and(|v| v != unsafe { libc::geteuid() })
                || gid.is_some_and(|v| v != unsafe { libc::getegid() })
            {
                return Err(os_error(libc::EPERM));
            }
            if let Some(size) = size {
                self.0.write(ino.0, 0, &[], Some(size))?;
            }
            let _io = self.0.io.lock().unwrap_or_else(|e| e.into_inner());
            let db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
            let mut entry = db.entry(ino.0)?.ok_or_else(|| os_error(libc::ENOENT))?;
            if let Some(mode) = mode {
                entry.mode = (mode & 0o777) as u16;
            }
            if let Some(time) = mtime {
                entry.modified = match time {
                    TimeOrNow::Now => now(),
                    TimeOrNow::SpecificTime(t) => {
                        t.duration_since(UNIX_EPOCH)?
                            .as_nanos()
                            .min(u64::MAX as u128) as u64
                    }
                };
            }
            db.save(&entry)?;
            Ok(entry)
        })();
        match result {
            Ok(e) => reply.attr(&TTL, &attr(&e)),
            Err(e) => reply.error(error(e)),
        }
    }
    fn mkdir(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        match self.create_entry(parent, name, Kind::Directory, mode, umask, request.pid()) {
            Ok(e) => reply.entry(&TTL, &attr(&e), Generation(0)),
            Err(e) => reply.error(error(e)),
        }
    }
    fn create(
        &self,
        request: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _: i32,
        reply: ReplyCreate,
    ) {
        match self.create_entry(parent, name, Kind::File, mode, umask, request.pid()) {
            Ok(e) => {
                *self
                    .0
                    .opens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(e.ino)
                    .or_default() += 1;
                reply.created(
                    &TTL,
                    &attr(&e),
                    Generation(0),
                    FileHandle(e.ino),
                    FopenFlags::FOPEN_DIRECT_IO,
                );
            }
            Err(e) => reply.error(error(e)),
        }
    }
    fn unlink(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        match self.remove(parent, name, false) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(error(e)),
        }
    }
    fn rmdir(&self, _: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        match self.remove(parent, name, true) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(error(e)),
        }
    }
    fn rename(
        &self,
        _: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        match self.rename_entry(parent, name, newparent, newname, flags) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(error(e)),
        }
    }
    fn open(&self, _: &Request, ino: INodeNo, _: OpenFlags, reply: ReplyOpen) {
        let _io = self.0.io.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = self.0.check_mutation() {
            reply.error(error(e));
            return;
        }
        match self.0.entry(ino.0) {
            Ok(_) => {
                *self
                    .0
                    .opens
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(ino.0)
                    .or_default() += 1;
                reply.opened(FileHandle(ino.0), FopenFlags::FOPEN_DIRECT_IO);
            }
            Err(e) => reply.error(error(e)),
        }
    }
    fn read(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.0.queue(IoRequest::Read {
            state: self.0.clone(),
            ino: ino.0,
            offset,
            size,
            reply,
        });
    }
    fn write(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        data: &[u8],
        _: WriteFlags,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        self.0.queue(IoRequest::Write {
            state: self.0.clone(),
            ino: ino.0,
            offset,
            data: data.to_vec(),
            reply,
        });
    }
    fn flush(&self, _: &Request, ino: INodeNo, _: FileHandle, _: LockOwner, reply: ReplyEmpty) {
        match self.0.file(ino.0).and_then(|f| Ok(f.sync_all()?)) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(error(e)),
        }
    }
    fn fsync(&self, _: &Request, ino: INodeNo, _: FileHandle, _: bool, reply: ReplyEmpty) {
        match self.0.file(ino.0).and_then(|f| Ok(f.sync_all()?)) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(error(e)),
        }
    }
    fn release(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        let mut opens = self.0.opens.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = opens.get_mut(&ino.0) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                opens.remove(&ino.0);
            }
        }
        reply.ok();
    }
    fn readdir(
        &self,
        _: &Request,
        ino: INodeNo,
        _: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let result = (|| {
            let db = self.0.catalog.lock().unwrap_or_else(|e| e.into_inner());
            let parent = db.entry(ino.0)?.context("Missing directory")?;
            let parent_path = parent.path.context("Unlinked directory")?;
            let ancestor = parent_path.rsplit_once('/').map(|p| p.0).unwrap_or("");
            let parent_ino = db.path(ancestor)?.map(|e| e.ino).unwrap_or(1);
            let mut listing = vec![
                (ino.0, FileType::Directory, ".".to_owned()),
                (parent_ino, FileType::Directory, "..".to_owned()),
            ];
            for entry in db.children(&parent_path)? {
                if let Some(path) = &entry.path {
                    if !path.is_empty()
                        && path.rsplit_once('/').map(|p| p.0).unwrap_or("") == parent_path
                    {
                        listing.push((
                            entry.ino,
                            kind(&entry),
                            path.rsplit('/').next().unwrap().to_owned(),
                        ));
                    }
                }
            }
            Ok::<_, anyhow::Error>(listing)
        })();
        match result {
            Ok(listing) => {
                for (index, (ino, kind, name)) in
                    listing.into_iter().enumerate().skip(offset as usize)
                {
                    if reply.add(INodeNo(ino), (index + 1) as u64, kind, name) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(e) => reply.error(error(e)),
        }
    }
    fn getxattr(&self, _: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        if name != "user.filenode.status" {
            reply.error(Errno::ENODATA);
            return;
        }
        match self.0.entry(ino.0) {
            Ok(e) => {
                let value = e.status().as_bytes();
                if size == 0 {
                    reply.size(value.len() as u32)
                } else if (size as usize) < value.len() {
                    reply.error(Errno::ERANGE)
                } else {
                    reply.data(value)
                }
            }
            Err(e) => reply.error(error(e)),
        }
    }
    fn statfs(&self, _: &Request, _: INodeNo, reply: ReplyStatfs) {
        // Cache capacity, never invented remote quota. statvfs follows the private
        // cache filesystem, not this mount, so it cannot recurse into FUSE.
        use std::ffi::CString;
        let Ok(path) = CString::new(self.0.cache.as_os_str().as_encoded_bytes()) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
            reply.error(Errno::EIO);
            return;
        }
        let stat = unsafe { stat.assume_init() };
        reply.statfs(
            stat.f_blocks,
            stat.f_bfree,
            stat.f_bavail,
            stat.f_files,
            stat.f_ffree,
            stat.f_bsize as u32,
            stat.f_namemax as u32,
            stat.f_frsize as u32,
        );
    }
}
