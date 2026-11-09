use super::{Identity, MAGIC};
use crate::{
    config::Config,
    jmap::Downloader,
    model::{Kind, LocalEntry, Node},
};
use ::windows::{
    core::{GUID, HRESULT, HSTRING, PCWSTR, PWSTR},
    Security::Cryptography::CryptographicBuffer,
    Storage::{Provider::*, StorageFolder},
    Win32::{
        Foundation::*,
        Security::{Authorization::ConvertSidToStringSidW, *},
        Storage::{CloudFilters::*, FileSystem::*},
        System::{
            Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent},
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
            IO::{GetOverlappedResult, OVERLAPPED},
        },
    },
};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    ffi::c_void,
    fs::{File, OpenOptions},
    mem::{offset_of, size_of},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread::JoinHandle,
};

const PROVIDER: GUID = GUID::from_u128(0xcc75133211ca4a94875f25c2661df4bf);
const CF_EOF: i64 = -1;
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn metadata_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(7)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?)
}
fn metadata_exclusive(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?)
}
fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}

/// Initialized on the engine's MTA thread, never on a WinUI dispatcher.
struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    fn new() -> Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

fn sid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        struct Token(HANDLE);
        impl Drop for Token {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let token = Token(token);
        let mut length = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut length);
        let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            length,
            &mut length,
        )?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let value = text.to_string();
        LocalFree(Some(HLOCAL(text.0.cast())));
        Ok(value?)
    }
}

fn root_id(config: &Config) -> Result<String> {
    Ok(format!("FileNode!{}!{}", sid()?, super::root_key(config)?))
}

/// Called before changing configuration. No provider registrations are changed.
pub fn validate_root(config: &Config) -> Result<()> {
    let _apartment = Apartment::new()?;
    let root = config.local_folder.canonicalize()?;
    let root_path = wide(&root);
    let mut volume = [0u16; 32768];
    let mut filesystem = [0u16; 64];
    unsafe {
        GetVolumePathNameW(PCWSTR(root_path.as_ptr()), &mut volume)?;
        GetVolumeInformationW(
            PCWSTR(volume.as_ptr()),
            None,
            None,
            None,
            None,
            Some(&mut filesystem),
        )?;
    }
    if String::from_utf16_lossy(
        &filesystem[..filesystem
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(filesystem.len())],
    ) != "NTFS"
    {
        bail!("Windows Files On-Demand requires a local NTFS folder");
    }
    // Windows rejects overlaps too. Preflight gives a useful error before any
    // persistent registration or conversion of user files takes place.
    let own_id = root_id(config)?;
    for registered in StorageProviderSyncRootManager::GetCurrentSyncRoots()? {
        let path = PathBuf::from(registered.Path()?.Path()?.to_string());
        let path = path.canonicalize().unwrap_or(path);
        if (root.starts_with(&path) || path.starts_with(&root)) && registered.Id()? != own_id {
            bail!("Files On-Demand cannot overlap another cloud provider's folder: {}. Select a separate FileNode folder.", path.display());
        }
    }
    for ancestor in root.ancestors() {
        let meta = std::fs::symlink_metadata(ancestor)?;
        if meta.file_type().is_symlink() {
            bail!("Cloud sync roots cannot traverse symbolic links");
        }
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            && !registered_root_matches(ancestor, &super::root_key(config)?)?
        {
            bail!(
                "Cloud sync roots cannot traverse another provider or a junction: {}",
                ancestor.display()
            );
        }
    }
    Ok(())
}

fn registered_root_matches(path: &Path, key: &str) -> Result<bool> {
    let file = metadata_file(path)?;
    registered_handle_matches(&file, key)
}
fn registered_handle_matches(file: &File, key: &str) -> Result<bool> {
    let mut buffer = vec![0u64; 1024];
    let mut returned = 0;
    let result = unsafe {
        CfGetSyncRootInfoByHandle(
            handle(file),
            CF_SYNC_ROOT_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            Some(&mut returned),
        )
    };
    if result.is_err() {
        return Ok(false);
    }
    let info = unsafe { &*buffer.as_ptr().cast::<CF_SYNC_ROOT_STANDARD_INFO>() };
    let start = offset_of!(CF_SYNC_ROOT_STANDARD_INFO, SyncRootIdentity);
    let length = info.SyncRootIdentityLength as usize;
    if start + length > returned as usize || length > 4096 {
        return Ok(false);
    }
    let identity =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(start), length) };
    Ok(identity == format!("{MAGIC}:{key}").as_bytes())
}

struct Placeholder {
    identity: Identity,
    in_sync: bool,
    pin: CF_PIN_STATE,
    on_disk: i64,
    modified: i64,
    size: u64,
    file_id: i64,
}
impl Placeholder {
    fn unchanged(&self) -> bool {
        self.in_sync || (self.modified == 0 && self.size == self.identity.node.size)
    }
}
fn placeholder(path: &Path) -> Result<Option<Placeholder>> {
    // CldFlt can hide REPARSE_POINT from ordinary metadata queries after
    // disconnect or full hydration. Query an OPEN_REPARSE_POINT handle even
    // when the projected attributes look like those of a regular file.
    let file = metadata_file(path)?;
    placeholder_handle(&file)
}
fn placeholder_handle(file: &File) -> Result<Option<Placeholder>> {
    let meta = file.metadata()?;
    let mut buffer = vec![0u64; 1024];
    let mut returned = 0;
    let result = unsafe {
        CfGetPlaceholderInfo(
            handle(file),
            CF_PLACEHOLDER_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            Some(&mut returned),
        )
    };
    if result.is_err() {
        return Ok(None);
    }
    let info = unsafe { &*buffer.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>() };
    let start = offset_of!(CF_PLACEHOLDER_STANDARD_INFO, FileIdentity);
    let length = info.FileIdentityLength as usize;
    if length > 4096 || start + length > returned as usize {
        return Ok(None);
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(start), length) };
    let Ok(identity) = Identity::decode(bytes) else {
        return Ok(None);
    };
    if !registered_handle_matches(file, &identity.root)? {
        return Ok(None);
    }
    Ok(Some(Placeholder {
        identity,
        in_sync: info.InSyncState == CF_IN_SYNC_STATE_IN_SYNC,
        pin: info.PinState,
        on_disk: info.OnDiskDataSize,
        modified: info.ModifiedDataSize,
        size: meta.len(),
        file_id: info.FileId,
    }))
}
pub fn is_owned(path: &Path) -> Result<bool> {
    Ok(placeholder(path)?.is_some())
}
pub fn committed_entry(path: &Path) -> Result<Option<LocalEntry>> {
    // Renames and pin changes can clear IN_SYNC without changing any data.
    // ModifiedDataSize distinguishes those from writes without hydrating a file.
    Ok(placeholder(path)?
        .filter(Placeholder::unchanged)
        .map(|p| p.identity.local))
}
pub fn matches_node(path: &Path, node: &Node, local: &LocalEntry) -> Result<bool> {
    Ok(placeholder(path)?.is_some_and(|p| {
        p.in_sync
            && p.identity.local == *local
            && p.identity.node.id == node.id
            && p.identity.node.blob_id == node.blob_id
    }))
}

fn mark_metadata_commit(path: &Path, node: &Node, expected: &LocalEntry) -> Result<()> {
    // An attribute-only, share-none handle does not trigger hydration. It also
    // excludes existing and new writers/deleters until this short commit ends.
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let current =
        placeholder_handle(&file)?.context("Cloud file disappeared during metadata commit")?;
    if !current.unchanged()
        || current.identity.local != *expected
        || current.identity.node.id != node.id
        || current.identity.node.blob_id != node.blob_id
    {
        bail!("Cloud file changed during metadata commit");
    }
    unsafe {
        CfSetInSyncState(
            handle(&file),
            CF_IN_SYNC_STATE_IN_SYNC,
            CF_SET_IN_SYNC_FLAG_NONE,
            None,
        )
        .context("Commit cloud file metadata")?;
    }
    Ok(())
}

fn replace_remote_version(path: &Path, identity: &Identity, bytes: &[u8]) -> Result<()> {
    // CfOpenFileWithOplock requests READ_DATA and can hydrate an online-only
    // file before it is updated. Use a share-none reparse handle for this
    // metadata-only replacement. No user data stream is opened.
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let previous =
        placeholder_handle(&file)?.context("Remote update requires an owned placeholder")?;
    if !previous.unchanged() {
        bail!("Cloud file changed before remote update; rescan required");
    }
    struct RestorePin<'a>(&'a File, CF_PIN_STATE);
    impl Drop for RestorePin<'_> {
        fn drop(&mut self) {
            if self.1 == CF_PIN_STATE_PINNED {
                unsafe {
                    let _ = CfSetPinState(handle(self.0), self.1, CF_SET_PIN_FLAG_NONE, None);
                }
            }
        }
    }
    let _restore = RestorePin(&file, previous.pin);
    if previous.pin == CF_PIN_STATE_PINNED {
        unsafe {
            CfSetPinState(
                handle(&file),
                CF_PIN_STATE_UNSPECIFIED,
                CF_SET_PIN_FLAG_NONE,
                None,
            )?;
        }
    }
    let mut flags = CF_UPDATE_FLAG_MARK_IN_SYNC;
    if previous.in_sync {
        flags |= CF_UPDATE_FLAG_VERIFY_IN_SYNC;
    }
    if identity.node.kind == Kind::File {
        flags |= CF_UPDATE_FLAG_DEHYDRATE;
    } else {
        flags |= CF_UPDATE_FLAG_DISABLE_ON_DEMAND_POPULATION;
    }
    let mut metadata = file_metadata(&identity.node);
    metadata.BasicInfo.FileAttributes = 0;
    unsafe {
        CfUpdatePlaceholder(
            handle(&file),
            Some(&metadata),
            Some(bytes.as_ptr().cast()),
            bytes.len() as u32,
            None,
            flags,
            None,
            None,
        )?;
    }
    Ok(())
}

pub fn is_registered(config: &Config) -> Result<bool> {
    let _apartment = Apartment::new()?;
    Ok(
        StorageProviderSyncRootManager::GetSyncRootInformationForId(&HSTRING::from(root_id(
            config,
        )?))
        .is_ok(),
    )
}

/// Detachment is deliberately explicit and lossless: Windows must already have
/// every byte. The user can request that through Explorer's native pin command.
pub fn check_detach(config: &Config) -> Result<()> {
    if !is_registered(config)? {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(&config.local_folder).follow_links(false) {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }
        if let Some(info) = placeholder(entry.path())? {
            if info.identity.node.kind == Kind::File
                && info.on_disk < info.identity.node.size as i64
            {
                bail!("Keep this folder on your device in Explorer and wait for downloads to finish before disabling Files On-Demand or stopping sync: {}", config.local_folder.display());
            }
        }
    }
    Ok(())
}

pub fn removed_roots(before: &Config, after: &Config) -> Result<Vec<Config>> {
    if after.files_on_demand {
        for config in after.folder_configs()? {
            validate_root(&config)?;
        }
    }
    if !before.files_on_demand {
        return Ok(Vec::new());
    }
    let keep: Vec<_> = if after.files_on_demand {
        after
            .folder_configs()?
            .iter()
            .map(super::root_key)
            .collect::<Result<_>>()?
    } else {
        Vec::new()
    };
    let mut removed = Vec::new();
    for root in before.folder_configs()? {
        if !keep.contains(&super::root_key(&root)?) {
            check_detach(&root)?;
            removed.push(root);
        }
    }
    Ok(removed)
}

pub fn detach(config: &Config) -> Result<()> {
    check_detach(config)?;
    let was_registered = is_registered(config)?;
    let _apartment = Apartment::new()?;
    let source = config.private_directory()?.join("state.sqlite");
    let store = if source.exists() {
        Some(crate::store::Store::open(config)?)
    } else {
        None
    };
    let baselines = store
        .as_ref()
        .map(|store| store.baselines())
        .transpose()?
        .unwrap_or_default();
    let mut committed_hashes = HashMap::new();
    // Deepest files first; retain data and all local edits. Reverting a partial
    // file is forbidden by the platform as an additional race protection.
    let mut entries = walkdir::WalkDir::new(&config.local_folder)
        .follow_links(false)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth()));
    for entry in entries {
        if entry.depth() > 0 && is_owned(entry.path())? {
            if entry.file_type().is_file() {
                let mut snapshot = crate::filesystem::open_snapshot(entry.path())?;
                if let Some(info) = placeholder(entry.path())? {
                    let mut file_info = BY_HANDLE_FILE_INFORMATION::default();
                    unsafe {
                        GetFileInformationByHandle(handle(&snapshot), &mut file_info)?;
                    }
                    let file_id =
                        ((file_info.nFileIndexHigh as u64) << 32) | file_info.nFileIndexLow as u64;
                    let relative = entry
                        .path()
                        .strip_prefix(&config.local_folder)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if file_id == info.file_id as u64
                        && info.unchanged()
                        && info.identity.local.hash.starts_with("cloud:")
                        && baselines.get(&relative).is_some_and(|b| {
                            b.local == info.identity.local
                                && b.remote_id == info.identity.node.id
                                && b.remote_blob == info.identity.node.blob_id
                        })
                    {
                        committed_hashes.insert(
                            relative,
                            crate::filesystem::hash_snapshot(&mut snapshot, u64::MAX)?,
                        );
                    }
                }
            }
            let file = Protected::open(entry.path(), true)?;
            unsafe {
                CfRevertPlaceholder(file.raw(), CF_REVERT_FLAG_NONE, None)?;
            }
        }
    }
    if was_registered {
        StorageProviderSyncRootManager::Unregister(&HSTRING::from(root_id(config)?))?;
    }
    // The old full-sync database may be stale. Merge the latest baselines back
    // from the external cloud database after all placeholders are ordinary files.
    // Replace synthetic hashes with actual hashes to avoid re-uploading materialized data.
    if let Some(store) = store {
        for (path, baseline) in baselines {
            if let Some(local) = committed_hashes.remove(&path) {
                let mut updated = baseline;
                updated.local = local;
                store.db.execute(
                    "UPDATE baseline SET value=?1 WHERE path=?2",
                    rusqlite::params![serde_json::to_string(&updated)?, path],
                )?;
            }
        }
        let legacy = config.local_folder.join(crate::filesystem::PRIVATE_DIR);
        crate::store::ensure_state_directory(&legacy)?;
        use fs2::FileExt;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(legacy.join("lock"))?;
        lock.try_lock_exclusive()?;
        let legacy_db = legacy.join("state.sqlite");
        if legacy_db.exists() {
            let old = rusqlite::Connection::open(&legacy_db)?;
            old.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        let staged = tempfile::NamedTempFile::new_in(&legacy)?.into_temp_path();
        std::fs::remove_file(&staged)?;
        store
            .db
            .execute("VACUUM INTO ?1", [staged.to_string_lossy().as_ref()])?;
        std::fs::rename(&staged, legacy_db)?;
        store.set_meta("cloud_detached", &true)?;
    }
    Ok(())
}

pub fn check_uninstall() -> Result<()> {
    let _apartment = Apartment::new()?;
    let prefix = format!("FileNode!{}!", sid()?);
    for root in StorageProviderSyncRootManager::GetCurrentSyncRoots()? {
        if root.Id()?.to_string().starts_with(&prefix) {
            bail!("Disable Files On-Demand in FileNode before uninstalling. Use Explorer's Always keep on this device command first so online-only files remain accessible.");
        }
    }
    Ok(())
}

struct Protected(HANDLE);
impl Protected {
    fn open(path: &Path, exclusive: bool) -> Result<Self> {
        let path = wide(path);
        let flags = CF_OPEN_FILE_FLAG_WRITE_ACCESS
            | if exclusive {
                CF_OPEN_FILE_FLAG_EXCLUSIVE
            } else {
                CF_OPEN_FILE_FLAG_NONE
            };
        Ok(Self(unsafe {
            CfOpenFileWithOplock(PCWSTR(path.as_ptr()), flags)
                .context("Acquire cloud file oplock")?
        }))
    }
    fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for Protected {
    fn drop(&mut self) {
        unsafe {
            CfCloseHandle(self.0);
        }
    }
}

struct Lease(HANDLE);
impl Lease {
    fn new(file: &Protected) -> Result<Self> {
        if !unsafe { CfReferenceProtectedHandle(file.0) } {
            bail!("File changed while acquiring its cloud snapshot");
        }
        Ok(Self(file.0))
    }
    fn verify(&self, expected: &LocalEntry) -> Result<()> {
        if expected.kind == Kind::Directory || expected.hash.starts_with("cloud:") {
            return Ok(());
        }
        use sha2::{Digest, Sha256};
        let handle = unsafe { CfGetWin32HandleFromProtectedHandle(self.0) };
        let mut hash = Sha256::new();
        let mut offset = 0u64;
        let mut buffer = [0u8; 65536];
        struct Event(HANDLE);
        impl Drop for Event {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let event = Event(unsafe { CreateEventW(None, true, false, None)? });
        while offset < expected.size {
            crate::runtime::check()?;
            let mut op = OVERLAPPED::default();
            unsafe {
                ResetEvent(event.0)?;
            }
            op.hEvent = event.0;
            op.Anonymous.Anonymous.Offset = offset as u32;
            op.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
            let mut count = 0;
            let length = (expected.size - offset).min(buffer.len() as u64) as usize;
            let result = unsafe {
                ReadFile(
                    handle,
                    Some(&mut buffer[..length]),
                    Some(&mut count),
                    Some(&mut op),
                )
            };
            if let Err(error) = result {
                if error.code() != HRESULT::from_win32(ERROR_IO_PENDING.0) {
                    return Err(error).context("Read protected snapshot");
                }
                unsafe {
                    GetOverlappedResult(handle, &op, &mut count, true)
                        .context("Complete protected snapshot read")?;
                }
            }
            if count == 0 {
                bail!("Local cloud file changed before commit");
            }
            hash.update(&buffer[..count as usize]);
            offset += count as u64;
        }
        if hex::encode(hash.finalize()) != expected.hash {
            bail!("Local cloud file changed before commit");
        }
        let mut info = FILE_STANDARD_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileStandardInfo,
                (&mut info as *mut FILE_STANDARD_INFO).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            )
            .context("Read protected file size")?;
        }
        if info.EndOfFile != expected.size as i64 {
            bail!("Local cloud file grew before commit");
        }
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        unsafe {
            CfReleaseProtectedHandle(self.0);
        }
    }
}

struct State {
    root: PathBuf,
    key: String,
    downloader: RwLock<Arc<dyn Downloader>>,
    requests: Mutex<HashMap<(i64, i64, i64, i64), PendingRequest>>,
    jobs: Mutex<Vec<JoinHandle<()>>>,
    stopping: AtomicBool,
    activity: crate::activity::Captured,
}
struct PendingRequest {
    cancel: Arc<AtomicBool>,
    offset: i64,
    length: i64,
}
pub struct Provider {
    state: Arc<State>,
    connection: CF_CONNECTION_KEY,
    _apartment: Apartment,
}
impl Provider {
    pub fn connect(config: &Config, downloader: Arc<dyn Downloader>) -> Result<Self> {
        validate_root(config).context("Validate Cloud Files root")?;
        let apartment = Apartment::new()?;
        let key = super::root_key(config)?;
        let canonical = config.local_folder.canonicalize()?;
        let canonical = canonical.to_string_lossy();
        let storage_path = canonical.strip_prefix(r"\\?\").unwrap_or(&canonical);
        let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(storage_path))?
            .join()
            .context("Open Cloud Files root through Windows Storage")?;
        let registration = StorageProviderSyncRootInfo::new()?;
        registration.SetId(&HSTRING::from(root_id(config)?))?;
        registration.SetPath(&folder)?;
        registration.SetDisplayNameResource(&HSTRING::from(format!(
            "FileNode — {}",
            config.remote_folder_path()?
        )))?;
        let executable = std::env::current_exe()?;
        let icon = executable
            .parent()
            .context("Executable path")?
            .join("Assets/filenode.ico");
        registration.SetIconResource(&HSTRING::from(if icon.exists() {
            icon.to_string_lossy().into_owned()
        } else {
            format!("{},0", executable.display())
        }))?;
        registration.SetProviderId(PROVIDER)?;
        registration.SetHydrationPolicy(StorageProviderHydrationPolicy::Progressive)?;
        registration.SetHydrationPolicyModifier(
            StorageProviderHydrationPolicyModifier::AutoDehydrationAllowed,
        )?;
        registration.SetPopulationPolicy(StorageProviderPopulationPolicy::AlwaysFull)?;
        registration.SetInSyncPolicy(
            StorageProviderInSyncPolicy::FileLastWriteTime
                | StorageProviderInSyncPolicy::FileCreationTime,
        )?;
        registration.SetHardlinkPolicy(StorageProviderHardlinkPolicy::None)?;
        registration.SetAllowPinning(true)?;
        registration.SetShowSiblingsAsGroup(true)?;
        registration.SetVersion(&HSTRING::from(env!("CARGO_PKG_VERSION")))?;
        registration
            .SetContext(&CryptographicBuffer::CreateFromByteArray(
                format!("{MAGIC}:{key}").as_bytes(),
            )?)
            .context("Set Cloud Files context")?;
        StorageProviderSyncRootManager::Register(&registration)
            .context("Register FileNode in Windows Explorer")?;
        let state = Arc::new(State {
            root: config.local_folder.clone(),
            key,
            downloader: RwLock::new(downloader),
            requests: Mutex::new(HashMap::new()),
            jobs: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
            activity: crate::activity::Scope::capture(),
        });
        let callbacks = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(fetch),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_CANCEL_FETCH_DATA,
                Callback: Some(cancel_fetch),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ];
        let root = wide(&config.local_folder);
        let connection = unsafe {
            CfConnectSyncRoot(
                PCWSTR(root.as_ptr()),
                callbacks.as_ptr(),
                Some(Arc::as_ptr(&state).cast()),
                CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH,
            )
        }
        .context("Connect the Windows Cloud Files provider")?;
        Ok(Self {
            state,
            connection,
            _apartment: apartment,
        })
    }

    pub fn publish(
        &self,
        relative: &str,
        node: &Node,
        local: Option<&LocalEntry>,
    ) -> Result<LocalEntry> {
        let target = crate::filesystem::checked_path(&self.state.root, relative)?;
        let identity = Identity::new(&self.state.key, node, local);
        let bytes = identity.encode()?;
        if target.exists() {
            if local.is_none() {
                replace_remote_version(&target, &identity, &bytes)?;
                return Ok(identity.local);
            }
            if let (Some(local), Some(old)) = (local, placeholder(&target)?) {
                if old.unchanged()
                    && old.identity.local == *local
                    && old.identity.node.id == node.id
                    && old.identity.node.blob_id == node.blob_id
                {
                    mark_metadata_commit(&target, node, local)?;
                    return Ok(old.identity.local);
                }
            }
            // An exclusive oplock plus VERIFY_IN_SYNC closes the race between
            // verification and replacing remote content. Never evict local edits.
            let file = Protected::open(&target, true)?;
            let lease = Lease::new(&file)?;
            let existing = placeholder(&target)?;
            if let Some(local) = local {
                lease.verify(local).context("Verify cloud file commit")?;
            }
            if let Some(old) = existing {
                if (local.is_none() || local.is_some_and(|l| l.hash.starts_with("cloud:")))
                    && !old.unchanged()
                {
                    bail!("Local cloud file changed; rescan required");
                }
                if local.is_some_and(|local| *local == old.identity.local)
                    && old.identity.node.id == node.id
                    && old.identity.node.blob_id == node.blob_id
                {
                    // A name or pin-state change does not change the blob.
                    // Updating its opaque identity unnecessarily can force the
                    // platform to hydrate an otherwise online-only file.
                    unsafe {
                        CfSetInSyncState(
                            file.raw(),
                            CF_IN_SYNC_STATE_IN_SYNC,
                            CF_SET_IN_SYNC_FLAG_NONE,
                            None,
                        )?;
                    }
                    return Ok(old.identity.local);
                }
                let mut flags =
                    CF_UPDATE_FLAG_MARK_IN_SYNC | CF_UPDATE_FLAG_DISABLE_ON_DEMAND_POPULATION;
                if old.in_sync {
                    flags |= CF_UPDATE_FLAG_VERIFY_IN_SYNC;
                }
                unsafe {
                    CfUpdatePlaceholder(
                        file.raw(),
                        None,
                        Some(bytes.as_ptr().cast()),
                        bytes.len() as u32,
                        None,
                        flags,
                        None,
                        None,
                    )?;
                }
            } else {
                if local.is_none() {
                    bail!("Cannot replace an uncommitted local file with a cloud placeholder");
                }
                unsafe {
                    CfConvertToPlaceholder(
                        file.raw(),
                        Some(bytes.as_ptr().cast()),
                        bytes.len() as u32,
                        CF_CONVERT_FLAG_MARK_IN_SYNC,
                        None,
                        None,
                    )
                    .context("Convert committed local file to a placeholder")?;
                }
            }
        } else {
            let name = wide(
                target
                    .file_name()
                    .map(Path::new)
                    .context("Placeholder name")?,
            );
            let parent = wide(target.parent().context("Placeholder directory")?);
            let mut entries = [CF_PLACEHOLDER_CREATE_INFO {
                RelativeFileName: PCWSTR(name.as_ptr()),
                FsMetadata: file_metadata(node),
                FileIdentity: bytes.as_ptr().cast(),
                FileIdentityLength: bytes.len() as u32,
                Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC
                    | CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION,
                ..Default::default()
            }];
            unsafe {
                CfCreatePlaceholders(
                    PCWSTR(parent.as_ptr()),
                    &mut entries,
                    CF_CREATE_FLAG_NONE,
                    None,
                )?;
            }
            entries[0].Result.ok()?;
        }
        Ok(identity.local)
    }

    /// Switch subsequent requests to the account's refreshed session and pool.
    pub fn set_downloader(&self, downloader: Arc<dyn Downloader>) {
        *self
            .state
            .downloader
            .write()
            .unwrap_or_else(|e| e.into_inner()) = downloader;
    }

    /// Reconcile Explorer's pin/unpin requests without changing server content.
    pub fn availability(&self) -> Result<()> {
        // Reap completed requests so the long-lived provider does not grow.
        self.state
            .jobs
            .lock()
            .unwrap()
            .retain(|job| !job.is_finished());
        for entry in walkdir::WalkDir::new(&self.state.root).follow_links(false) {
            crate::runtime::check()?;
            let entry = entry?;
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            if crate::filesystem::ignored(
                &entry
                    .path()
                    .strip_prefix(&self.state.root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            ) {
                continue;
            }
            let Some(info) = placeholder(entry.path())? else {
                continue;
            };
            if !info.in_sync {
                continue;
            }
            let result: Result<()> = (|| {
                if info.pin == CF_PIN_STATE_PINNED && info.on_disk < info.identity.node.size as i64
                {
                    let file = Protected::open(entry.path(), false)?;
                    unsafe {
                        CfHydratePlaceholder(file.raw(), 0, CF_EOF, CF_HYDRATE_FLAG_NONE, None)?;
                    }
                } else if info.pin == CF_PIN_STATE_UNPINNED && info.on_disk > 0 {
                    let file = metadata_exclusive(entry.path())?;
                    // CfDehydratePlaceholder refuses dirty files, pinned files and
                    // conflicting opens; a failed eviction keeps all existing data.
                    unsafe {
                        CfDehydratePlaceholder(
                            handle(&file),
                            0,
                            CF_EOF,
                            CF_DEHYDRATE_FLAG_NONE,
                            None,
                        )?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let busy = error
                    .downcast_ref::<::windows::core::Error>()
                    .is_some_and(|error| {
                        [
                            ERROR_SHARING_VIOLATION,
                            ERROR_LOCK_VIOLATION,
                            ERROR_CLOUD_FILE_IN_USE,
                        ]
                        .iter()
                        .any(|code| error.code() == HRESULT::from_win32(code.0))
                    })
                    || error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| matches!(error.raw_os_error(), Some(32 | 33 | 391)));
                if !busy {
                    return Err(error);
                }
                crate::activity::notice(
                    &entry
                        .path()
                        .strip_prefix(&self.state.root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    "check",
                    "waiting",
                );
            }
        }
        Ok(())
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.state.stopping.store(true, Ordering::SeqCst);
        for cancel in self.state.requests.lock().unwrap().values() {
            cancel.cancel.store(true, Ordering::SeqCst);
        }
        unsafe {
            let _ = CfDisconnectSyncRoot(self.connection);
        }
        for job in self.state.jobs.lock().unwrap().drain(..) {
            let _ = job.join();
        }
        // Registrations survive normal process exit, network changes and updates.
        // Removing them here would orphan online-only files.
    }
}

fn file_metadata(node: &Node) -> CF_FS_METADATA {
    let mut metadata = CF_FS_METADATA {
        FileSize: node.size as i64,
        ..Default::default()
    };
    metadata.BasicInfo.FileAttributes = if node.kind == Kind::Directory {
        FILE_ATTRIBUTE_DIRECTORY.0
    } else {
        FILE_ATTRIBUTE_NORMAL.0
    };
    metadata
}

#[derive(Clone, Copy)]
struct Request {
    connection: CF_CONNECTION_KEY,
    transfer: i64,
    request: i64,
    offset: i64,
    length: i64,
}
impl Request {
    fn transfer(
        &self,
        offset: i64,
        length: i64,
        buffer: *const c_void,
        status: NTSTATUS,
    ) -> Result<()> {
        let info = CF_OPERATION_INFO {
            StructSize: size_of::<CF_OPERATION_INFO>() as u32,
            Type: CF_OPERATION_TYPE_TRANSFER_DATA,
            ConnectionKey: self.connection,
            TransferKey: self.transfer,
            RequestKey: self.request,
            ..Default::default()
        };
        let mut params = CF_OPERATION_PARAMETERS {
            ParamSize: (offset_of!(CF_OPERATION_PARAMETERS, Anonymous)
                + size_of::<CF_OPERATION_PARAMETERS_0_0>()) as u32,
            Anonymous: CF_OPERATION_PARAMETERS_0 {
                TransferData: CF_OPERATION_PARAMETERS_0_0 {
                    Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                    CompletionStatus: status,
                    Buffer: buffer,
                    Offset: offset,
                    Length: length,
                },
            },
        };
        unsafe {
            CfExecute(&info, &mut params)?;
        }
        Ok(())
    }
    fn fail(&self, status: NTSTATUS) {
        let _ = self.transfer(self.offset, self.length, std::ptr::null(), status);
    }
    fn fail_remaining(&self, completed: i64, status: NTSTATUS) {
        let end = self.offset.saturating_add(self.length);
        let offset = completed.max(self.offset).min(end);
        if offset < end {
            let _ = self.transfer(offset, end - offset, std::ptr::null(), status);
        }
    }
}

unsafe extern "system" fn fetch(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    // No unwind may cross the Windows callback boundary. A copied request owns
    // everything it uses after this callback returns; CF buffers are ephemeral.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        fetch_inner(&*info, &*params)
    }));
}
unsafe fn fetch_inner(info: &CF_CALLBACK_INFO, params: &CF_CALLBACK_PARAMETERS) {
    let fetch = params.Anonymous.FetchData;
    let mut request = Request {
        connection: info.ConnectionKey,
        transfer: info.TransferKey,
        request: info.RequestKey,
        offset: fetch.RequiredFileOffset,
        length: fetch.RequiredLength,
    };
    let context = info.CallbackContext.cast::<State>();
    Arc::increment_strong_count(context);
    let state = Arc::from_raw(context);
    if state.stopping.load(Ordering::SeqCst)
        || info.FileIdentity.is_null()
        || info.FileIdentityLength > 4096
    {
        request.fail(STATUS_CLOUD_FILE_PROVIDER_NOT_RUNNING);
        return;
    }
    let bytes = std::slice::from_raw_parts(
        info.FileIdentity.cast::<u8>(),
        info.FileIdentityLength as usize,
    );
    let Ok(identity) = Identity::decode(bytes) else {
        request.fail(STATUS_CLOUD_FILE_UNSUCCESSFUL);
        return;
    };
    if request.length == CF_EOF
        && request.offset >= 0
        && request.offset as u64 <= identity.node.size
    {
        request.length = (identity.node.size - request.offset as u64) as i64;
    }
    if identity.root != state.key
        || request.offset < 0
        || request.length < 0
        || (request.offset as u64)
            .checked_add(request.length as u64)
            .is_none_or(|end| end > identity.node.size)
    {
        request.fail(STATUS_CLOUD_FILE_UNSUCCESSFUL);
        return;
    }
    let key = (
        request.transfer,
        request.request,
        request.offset,
        request.length,
    );
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut requests = state.requests.lock().unwrap();
        if requests.len() >= 16 || requests.contains_key(&key) {
            drop(requests);
            request.fail(STATUS_CLOUD_FILE_REQUEST_ABORTED);
            return;
        }
        requests.insert(
            key,
            PendingRequest {
                cancel: cancel.clone(),
                offset: request.offset,
                length: request.length,
            },
        );
    }
    let file_path =
        callback_relative_path(info, &state.root).unwrap_or_else(|| identity.node.name.clone());
    let worker = state.clone();
    let job = std::thread::Builder::new()
        .name("cloud-hydration".into())
        .spawn(move || {
            let mut completed = request.offset;
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let _scope = worker.activity.enter();
                    // CF transfers must be page aligned except for the last page at EOF.
                    // Coalesce short network reads before sending them to the platform.
                    let start = (request.offset as u64 / 4096) * 4096;
                    let end = (request.offset as u64 + request.length as u64)
                        .div_ceil(4096)
                        .saturating_mul(4096)
                        .min(identity.node.size);
                    let mut activity =
                        crate::activity::FileActivity::begin(&file_path, "download", end - start);
                    let progress = crate::activity::Progress::current();
                    let mut offset = start;
                    let mut pending = Vec::with_capacity(65536);
                    let downloader = worker.downloader.read().unwrap_or_else(|e| e.into_inner()).clone();
                    downloader.read_range(
                        &identity.node,
                        start,
                        end - start,
                        &cancel,
                        &mut |data| {
                            if worker.stopping.load(Ordering::SeqCst)
                                || cancel.load(Ordering::SeqCst)
                            {
                                bail!("Hydration cancelled");
                            }
                            pending.extend_from_slice(data);
                            let amount = if offset + pending.len() as u64 == identity.node.size {
                                pending.len()
                            } else {
                                pending.len() / 4096 * 4096
                            };
                            if amount > 0 {
                                request.transfer(
                                    offset as i64,
                                    amount as i64,
                                    pending.as_ptr().cast(),
                                    STATUS_SUCCESS,
                                )?;
                                offset += amount as u64;
                                completed = offset as i64;
                                pending.drain(..amount);
                                progress.set(offset - start);
                                unsafe {
                                    let _ = CfReportProviderProgress(
                                        request.connection,
                                        request.transfer,
                                        (end - start) as i64,
                                        (offset - start) as i64,
                                    );
                                }
                            }
                            Ok(())
                        },
                    )?;
                    if !pending.is_empty() || offset != end {
                        bail!("Incomplete cloud file range");
                    }
                    activity.finish("synced");
                    Ok(())
                }));
            if !matches!(result, Ok(Ok(()))) {
                request.fail_remaining(completed, if cancel.load(Ordering::SeqCst) {
                    STATUS_CANCELLED
                } else if matches!(&result, Ok(Err(error)) if error.downcast_ref::<crate::auth::SignInRequired>().is_some()) {
                    STATUS_CLOUD_FILE_AUTHENTICATION_FAILED
                } else {
                    STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE
                });
                if let Ok(Err(error)) = result {
                    crate::logger::event(
                        crate::logger::Level::Warning,
                        310,
                        "cloud_hydration_retry",
                        Some(&worker.root.to_string_lossy()),
                        &format!("{}: {error:#}", identity.node.name),
                    );
                }
            }
            worker.requests.lock().unwrap().remove(&key);
        });
    match job {
        Ok(job) => state.jobs.lock().unwrap().push(job),
        Err(_) => {
            state.requests.lock().unwrap().remove(&key);
            request.fail(STATUS_CLOUD_FILE_UNSUCCESSFUL);
        }
    }
}
unsafe extern "system" fn cancel_fetch(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let info = &*info;
        let state = &*info.CallbackContext.cast::<State>();
        for (key, pending) in state.requests.lock().unwrap().iter() {
            if key.0 != info.TransferKey || key.1 != info.RequestKey {
                continue;
            }
            let range = (*params).Anonymous.Cancel.Anonymous.FetchData;
            let end = if range.Length == CF_EOF {
                i64::MAX
            } else {
                range.FileOffset.saturating_add(range.Length)
            };
            // A partial cancellation can leave another reader waiting for the
            // rest. Sending surplus immutable data is allowed; abort only when
            // the whole outstanding request is no longer needed.
            if range.FileOffset <= pending.offset
                && end >= pending.offset.saturating_add(pending.length)
            {
                pending.cancel.store(true, Ordering::SeqCst);
            }
        }
    }));
}

unsafe fn callback_relative_path(info: &CF_CALLBACK_INFO, root: &Path) -> Option<String> {
    if info.NormalizedPath.is_null() || info.VolumeDosName.is_null() {
        return None;
    }
    let name = info.NormalizedPath.to_string().ok()?;
    let volume = info.VolumeDosName.to_string().ok()?;
    let full = if name.get(1..2) == Some(":") || name.starts_with(r"\\?\") {
        name
    } else {
        format!("{volume}{name}")
    };
    let root = root.canonicalize().ok()?;
    let root = root.to_string_lossy();
    let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
    let full = full.strip_prefix(r"\\?\").unwrap_or(&full);
    if !full
        .to_lowercase()
        .starts_with(&(root.to_lowercase() + "\\"))
    {
        return None;
    }
    Some(full[root.len() + 1..].replace('\\', "/"))
}

#[cfg(test)]
mod tests;
