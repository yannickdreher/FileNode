use super::*;
use std::sync::atomic::AtomicUsize;
struct Bytes {
    data: Vec<u8>,
    requests: AtomicUsize,
    fail_after: AtomicUsize,
    offsets: Mutex<Vec<u64>>,
}
impl Downloader for Bytes {
    fn read_range(
        &self,
        _: &Node,
        offset: u64,
        length: u64,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.offsets.lock().unwrap().push(offset);
        let mut emitted = 0;
        for part in self.data[offset as usize..(offset + length) as usize].chunks(1777) {
            if cancel.load(Ordering::SeqCst) {
                bail!("Cancelled test request");
            }
            let limit = self.fail_after.load(Ordering::SeqCst);
            if limit > 0 && emitted >= limit {
                bail!("Simulated interrupted range download");
            }
            consume(part)?;
            emitted += part.len();
        }
        Ok(())
    }
}
struct Cleanup(Config);
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(_apartment) = Apartment::new() {
            if let Ok(id) = root_id(&self.0) {
                let _ = StorageProviderSyncRootManager::Unregister(&HSTRING::from(id));
            }
        }
    }
}

#[test]
#[ignore = "Reconnects a persisted Cloud Files registration and migrates state in a fresh process"]
fn native_registration_survives_process_restart() -> Result<()> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    if let Some(root) = std::env::var_os("FILENODE_TEST_RESTART_ROOT") {
        let root = PathBuf::from(root).canonicalize()?;
        if !root.starts_with(fixtures.canonicalize()?) {
            bail!("Restart fixture must remain inside the isolated cloud test directory");
        }
        let config = Config {
            local_folder: root.clone(),
            username: "process-restart@example.org".into(),
            files_on_demand: true,
            ..Config::default()
        };
        let bytes = Arc::new(Bytes {
            data: b"persisted server contents".to_vec(),
            requests: AtomicUsize::new(0),
            fail_after: AtomicUsize::new(0),
            offsets: Mutex::new(Vec::new()),
        });
        let node = Node {
            id: "restart-node".into(),
            name: "persisted.txt".into(),
            parent_id: None,
            blob_id: Some("restart-blob".into()),
            kind: Kind::File,
            size: bytes.data.len() as u64,
            role: None,
        };
        let legacy = config.legacy_cloud_directory()?;
        match std::env::var("FILENODE_TEST_RESTART_PHASE")?.as_str() {
            "prepare" => {
                let provider = Provider::connect(&config, bytes.clone())?;
                let local = provider.publish("persisted.txt", &node, None)?;
                let store = crate::store::Store::open(&config)?;
                store.record("persisted.txt", &local, &node)?;
                std::fs::create_dir_all(&legacy)?;
                store.db.execute(
                    "VACUUM INTO ?1",
                    [legacy.join("state.sqlite").to_string_lossy().as_ref()],
                )?;
                drop(store);
                std::fs::remove_file(config.private_directory()?.join("state.sqlite"))?;
                assert_eq!(bytes.requests.load(Ordering::SeqCst), 0);
                // Leave the Explorer registration and online-only file intact,
                // as the old executable does when quitting for an update.
            }
            "restart" => {
                assert!(legacy.join("state.sqlite").is_file());
                assert!(is_registered(&config)?, "persisted registration was lost");
                // The first post-update store open precedes provider connection.
                let store = crate::store::Store::open(&config)?;
                let baseline = store.baselines()?.remove("persisted.txt").unwrap();
                assert_eq!(baseline.remote_id, node.id);
                assert!(!legacy.exists(), "legacy state was not migrated");
                assert_eq!(
                    committed_entry(&root.join("persisted.txt"))?,
                    Some(baseline.local.clone())
                );
                let _provider = Provider::connect(&config, bytes.clone())?;
                assert_eq!(bytes.requests.load(Ordering::SeqCst), 0);
                assert_eq!(std::fs::read(root.join("persisted.txt"))?, bytes.data);
                assert_eq!(
                    committed_entry(&root.join("persisted.txt"))?,
                    Some(baseline.local)
                );
            }
            _ => bail!("Unknown restart test phase"),
        }
        return Ok(());
    }

    let temp = tempfile::tempdir_in(fixtures)?;
    let root = temp.path().join("CloudFiles");
    std::fs::create_dir(&root)?;
    let _cleanup = Cleanup(Config {
        local_folder: root.clone(),
        username: "process-restart@example.org".into(),
        files_on_demand: true,
        ..Config::default()
    });
    for phase in ["prepare", "restart"] {
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--ignored",
                "--exact",
                "cloud::windows::tests::native_registration_survives_process_restart",
                "--test-threads=1",
            ])
            .env("FILENODE_TEST_RESTART_ROOT", &root)
            .env("FILENODE_TEST_RESTART_PHASE", phase)
            .env("FILENODE_LOG", "off")
            .output()?;
        assert!(
            output.status.success(),
            "Restart phase {phase} failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[test]
#[ignore = "Exercises Windows Runtime root queries across short-lived worker threads"]
fn native_root_queries_survive_worker_restarts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let config = Config {
        local_folder: temp.path().to_owned(),
        username: "no-account-access@example.org".into(),
        ..Config::default()
    };
    for iteration in 0..32 {
        let config = config.clone();
        std::thread::spawn(move || -> Result<()> {
            for _ in 0..4 {
                assert!(!is_registered(&config)?);
                validate_root(&config)?;
            }
            Ok(())
        })
        .join()
        .expect("Root query worker panicked")?;
        if iteration == 0 {
            // Real retries leave enough idle time for COM's unused-DLL sweep.
            // A tight loop alone can hide a stale cached Windows factory.
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    }
    Ok(())
}

#[test]
#[ignore = "Migrates legacy metadata marked as another provider's real Cloud Files placeholders"]
fn native_migration_accepts_cloud_tags_without_following_redirects() -> Result<()> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    let temp = tempfile::tempdir_in(fixtures)?;
    let root = temp.path().join("OtherCloud");
    let selected = root.join("Documents");
    std::fs::create_dir_all(&selected)?;
    let config = Config {
        local_folder: selected.clone(),
        username: "migration@example.org".into(),
        ..Config::default()
    };
    let legacy = selected.join(crate::filesystem::PRIVATE_DIR);
    {
        let store = crate::store::Store::open(&config)?;
        std::fs::create_dir_all(legacy.join("history"))?;
        std::fs::write(
            legacy.join("history/previous.txt"),
            b"previous local contents",
        )?;
        store.set_meta("migration_marker", &"retained")?;
        store.db.execute(
            "VACUUM INTO ?1",
            [legacy.join("state.sqlite").to_string_lossy().as_ref()],
        )?;
    }
    std::fs::remove_file(config.private_directory()?.join("state.sqlite"))?;
    let other = Config {
        local_folder: root,
        files_on_demand: true,
        ..config.clone()
    };
    let _cleanup = Cleanup(other.clone());
    let _provider = Provider::connect(&other, Arc::new(MemoryBytes::default()))?;
    for path in [&legacy, &legacy.join("state.sqlite")] {
        let file = Protected::open(path, true)?;
        let lease = Lease::new(&file)?;
        let identity = b"another-provider-owned-placeholder";
        unsafe {
            CfConvertToPlaceholder(
                lease.raw(),
                Some(identity.as_ptr().cast()),
                identity.len() as u32,
                CF_CONVERT_FLAG_MARK_IN_SYNC,
                None,
                None,
            )?;
        }
        assert!(is_cloud_placeholder(path)?);
        assert!(
            !is_owned(path)?,
            "migration must not assume other providers use FileNode's identity"
        );
    }
    assert!(
        !is_registered(&config)?,
        "another provider's enclosing root is not our root"
    );
    let migrated = crate::store::Store::open(&config)?;
    assert_eq!(
        migrated.get_meta::<String>("migration_marker")?.as_deref(),
        Some("retained")
    );
    assert!(!legacy.exists());
    assert!(
        walkdir::WalkDir::new(config.private_directory()?.join("history"))
            .into_iter()
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name() == "previous.txt"
                && std::fs::read(entry.path()).unwrap() == b"previous local contents")
    );
    // A directory junction is a namespace redirect, never a cloud placeholder.
    assert_eq!(
        unsafe {
            CfGetPlaceholderStateFromAttributeTag(
                FILE_ATTRIBUTE_REPARSE_POINT.0,
                0xA0000003, // IO_REPARSE_TAG_MOUNT_POINT
            )
        },
        CF_PLACEHOLDER_STATE_NO_STATES
    );
    Ok(())
}

#[test]
#[ignore = "Registers isolated Cloud Files roots to test mixed full-sync/cloud profiles"]
fn mixed_profiles_validate_and_register_only_enabled_folders() -> Result<()> {
    use crate::config::SyncFolder;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    let temp = tempfile::tempdir_in(fixtures)?;
    let other = temp.path().join("AnotherCloud");
    let full = other.join("Documents");
    let cloud = temp.path().join("Videos");
    std::fs::create_dir_all(&full)?;
    std::fs::create_dir(&cloud)?;
    let other_config = Config {
        local_folder: other,
        username: "test@example.org".into(),
        files_on_demand: true,
        ..Config::default()
    };
    let _other_cleanup = Cleanup(other_config.clone());
    let bytes = Arc::new(Bytes {
        data: Vec::new(),
        requests: AtomicUsize::new(0),
        fail_after: AtomicUsize::new(0),
        offsets: Mutex::new(Vec::new()),
    });
    let _other_provider = Provider::connect(&other_config, bytes)?;
    let mut profile = Config {
        username: "test@example.org".into(),
        file_settle_seconds: 0,
        sync_folders: Some(vec![
            SyncFolder {
                local_folder: full.clone(),
                remote_path: "Documents".into(),
                files_on_demand: Some(false),
            },
            SyncFolder {
                local_folder: cloud.clone(),
                remote_path: "Videos".into(),
                files_on_demand: Some(true),
            },
        ]),
        ..Config::default()
    };
    profile.validate()?;
    let mut before = profile.clone();
    before.sync_folders.as_mut().unwrap()[1].files_on_demand = Some(false);
    assert!(removed_roots(&before, &profile)?.is_empty());
    let roots = profile.folder_configs()?;
    let _cloud_cleanup = Cleanup(roots[1].clone());
    let mut invalid = profile.clone();
    invalid.sync_folders.as_mut().unwrap()[0].files_on_demand = Some(true);
    assert!(
        removed_roots(&profile, &invalid).is_err(),
        "enabled roots must still reject overlaps"
    );
    std::fs::write(full.join("local.txt"), b"full local sync")?;
    let mut remote = MemoryRemote::default();
    let videos = remote.seed("Videos", None, None);
    remote.seed("remote.txt", Some(&videos.id), Some("cloud contents"));
    let mut group = crate::group::SyncGroup::new(&profile)?;
    let report = group.poll(&mut remote, &AtomicBool::new(false), &mut |_| {})?;
    assert_eq!(report.failed_folders, 0);
    assert_eq!(report.uploaded, 1);
    assert!(
        !is_registered(&roots[0])?,
        "a full-sync folder was registered as cloud storage"
    );
    assert!(is_registered(&roots[1])?);
    assert!(is_registered(&other_config)?);
    assert!(is_owned(&cloud.join("remote.txt"))?);
    assert_eq!(
        remote.downloads, 0,
        "an online-only placeholder was downloaded eagerly"
    );
    let mut disabled = profile.clone();
    disabled.sync_folders.as_mut().unwrap()[1].files_on_demand = Some(false);
    assert!(
        removed_roots(&profile, &disabled).is_err(),
        "online-only contents must be kept before disabling"
    );
    assert_eq!(std::fs::read(cloud.join("remote.txt"))?, b"cloud contents");
    drop(group);
    let removed = removed_roots(&profile, &disabled)?;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].local_folder, cloud);
    detach(&removed[0])?;
    assert!(!is_registered(&roots[1])?);
    assert!(
        is_registered(&other_config)?,
        "unrelated cloud root was detached"
    );
    profile.sync_folders.as_mut().unwrap()[1].files_on_demand = Some(false);
    assert!(!profile.has_cloud_folders());
    Ok(())
}
#[derive(Default)]
struct MemoryRemote {
    nodes: Vec<Node>,
    bytes: Arc<MemoryBytes>,
    revision: usize,
    uploads: usize,
    downloads: usize,
}
#[derive(Default)]
struct MemoryBytes {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
    reads: AtomicUsize,
}
impl Downloader for MemoryBytes {
    fn read_range(
        &self,
        node: &Node,
        offset: u64,
        length: u64,
        cancel: &AtomicBool,
        consume: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<()> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let data = self.blobs.lock().unwrap()[node.blob_id.as_ref().unwrap()].clone();
        for bytes in data[offset as usize..(offset + length) as usize].chunks(16381) {
            if cancel.load(Ordering::SeqCst) {
                bail!("Cancelled");
            }
            consume(bytes)?;
        }
        Ok(())
    }
}
impl MemoryRemote {
    fn state(&self) -> String {
        self.revision.to_string()
    }
    fn seed(&mut self, name: &str, parent: Option<&str>, contents: Option<&str>) -> Node {
        let mut state = self.state();
        let blob = contents.map(|data| {
            let key = uuid::Uuid::new_v4().to_string();
            self.bytes
                .blobs
                .lock()
                .unwrap()
                .insert(key.clone(), data.as_bytes().to_vec());
            key
        });
        crate::jmap::Remote::create(
            self,
            name,
            parent,
            if contents.is_some() {
                Kind::File
            } else {
                Kind::Directory
            },
            blob.as_deref(),
            contents.map_or(0, |s| s.len() as u64),
            &mut state,
        )
        .unwrap()
    }
}
impl crate::jmap::Remote for MemoryRemote {
    fn downloader(&self) -> Option<Arc<dyn Downloader>> {
        Some(self.bytes.clone())
    }
    fn account_id(&self) -> &str {
        "cloud-test-account"
    }
    fn current_state(&self) -> Result<String> {
        Ok(self.state())
    }
    fn snapshot(&mut self, _: Option<&crate::model::Snapshot>) -> Result<crate::model::Snapshot> {
        Ok(crate::model::Snapshot {
            state: self.state(),
            nodes: self.nodes.clone(),
        })
    }
    fn upload(&mut self, path: &Path, _: u64) -> Result<String> {
        self.uploads += 1;
        let id = uuid::Uuid::new_v4().to_string();
        self.bytes
            .blobs
            .lock()
            .unwrap()
            .insert(id.clone(), std::fs::read(path)?);
        Ok(id)
    }
    fn download(&mut self, node: &Node, path: &Path) -> Result<()> {
        self.downloads += 1;
        std::fs::write(
            path,
            &self.bytes.blobs.lock().unwrap()[node.blob_id.as_ref().unwrap()],
        )?;
        Ok(())
    }
    fn create_trash(&mut self, parent: Option<&str>, state: &mut String) -> Result<Node> {
        let mut node = self.create(".Trash", parent, Kind::Directory, None, 0, state)?;
        node.role = Some("trash".into());
        self.nodes
            .iter_mut()
            .find(|n| n.id == node.id)
            .unwrap()
            .role = node.role.clone();
        Ok(node)
    }
    fn create(
        &mut self,
        name: &str,
        parent: Option<&str>,
        kind: Kind,
        blob: Option<&str>,
        size: u64,
        state: &mut String,
    ) -> Result<Node> {
        if *state != self.state() {
            bail!("stateMismatch");
        }
        let node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            parent_id: parent.map(str::to_owned),
            blob_id: blob.map(str::to_owned),
            kind,
            size,
            role: None,
        };
        self.nodes.push(node.clone());
        self.revision += 1;
        *state = self.state();
        Ok(node)
    }
    fn update(
        &mut self,
        node: &Node,
        name: &str,
        parent: Option<&str>,
        blob: Option<(&str, u64)>,
        state: &mut String,
    ) -> Result<Node> {
        if *state != self.state() {
            bail!("stateMismatch");
        }
        let updated = self.nodes.iter_mut().find(|n| n.id == node.id).unwrap();
        updated.name = name.into();
        updated.parent_id = parent.map(str::to_owned);
        if let Some((id, size)) = blob {
            updated.blob_id = Some(id.into());
            updated.size = size;
        }
        let updated = updated.clone();
        self.revision += 1;
        *state = self.state();
        Ok(updated)
    }
}

#[test]
#[ignore = "Exercises the real Windows Cloud Files driver and Explorer registration"]
fn native_cloud_sync_round_trip_and_detach() -> Result<()> {
    use crate::jmap::Remote;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    let temp = tempfile::tempdir_in(fixtures)?;
    let root = temp.path().join("CloudFiles");
    std::fs::create_dir(&root)?;
    let mut config = Config {
        local_folder: root.clone(),
        username: "test@example.org".into(),
        files_on_demand: true,
        file_settle_seconds: 0,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.clone());
    let mut remote = MemoryRemote::default();
    let folder = remote.seed("CloudFiles", None, None);
    let nested = remote.seed("nested", Some(&folder.id), None);
    let a = remote.seed(
        "remote.txt",
        Some(&nested.id),
        Some("initial server contents"),
    );
    let b = remote.seed(
        "rename.txt",
        Some(&folder.id),
        Some("never hydrated to rename"),
    );
    let unseen = remote.seed(
        "unseen.txt",
        Some(&folder.id),
        Some("old content not opened"),
    );
    remote.seed("trash.txt", Some(&folder.id), Some("online trash test"));
    let mut store = crate::store::Store::open(&config)?;
    let mut scanner = crate::scanner::LocalScanner::new(&root, 900);
    let cancel = AtomicBool::new(false);
    let run = |config: &Config,
               store: &mut crate::store::Store,
               remote: &mut MemoryRemote,
               scanner: &mut crate::scanner::LocalScanner| {
        scanner.invalidate();
        crate::engine::sync_with_scanner(config, store, remote, &cancel, &mut |_| {}, scanner)
    };
    run(&config, &mut store, &mut remote, &mut scanner)
        .context("Initial placeholder population")?;
    assert_eq!(remote.downloads, 0);
    assert_eq!(remote.bytes.reads.load(Ordering::SeqCst), 0);
    remote
        .bytes
        .blobs
        .lock()
        .unwrap()
        .insert("unseen-v2".into(), b"new unread content".to_vec());
    let mut state = remote.state();
    remote.update(
        &unseen,
        &unseen.name,
        Some(&folder.id),
        Some(("unseen-v2", 18)),
        &mut state,
    )?;
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        remote.bytes.reads.load(Ordering::SeqCst),
        0,
        "updating an online-only file fetched obsolete content"
    );
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(report.uploaded + report.downloaded, 0);
    assert_eq!(
        std::fs::read(root.join("nested/remote.txt"))?,
        b"initial server contents"
    );
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        report.uploaded + report.downloaded,
        0,
        "hydration fed back into synchronization"
    );
    std::fs::write(root.join("local.txt"), b"new local content")?;
    assert_eq!(
        run(&config, &mut store, &mut remote, &mut scanner)?.uploaded,
        1
    );
    let reads = remote.bytes.reads.load(Ordering::SeqCst);
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        report.uploaded + report.downloaded,
        0,
        "own upload was downloaded again"
    );
    assert_eq!(reads, remote.bytes.reads.load(Ordering::SeqCst));
    // Remote edits invalidate cached content without downloading it eagerly.
    remote
        .bytes
        .blobs
        .lock()
        .unwrap()
        .insert("revision2".into(), b"new server version".to_vec());
    let mut state = remote.state();
    remote.update(
        &a,
        &a.name,
        a.parent_id.as_deref(),
        Some(("revision2", 18)),
        &mut state,
    )?;
    run(&config, &mut store, &mut remote, &mut scanner).context("Remote placeholder update")?;
    assert_eq!(reads, remote.bytes.reads.load(Ordering::SeqCst));
    assert_eq!(
        std::fs::read(root.join("nested/remote.txt"))?,
        b"new server version"
    );
    // Rename a still-online-only file in both directions, retaining its ID.
    let reads = remote.bytes.reads.load(Ordering::SeqCst);
    std::fs::rename(root.join("rename.txt"), root.join("renamed.txt"))?;
    assert_eq!(
        reads,
        remote.bytes.reads.load(Ordering::SeqCst),
        "the Windows rename itself hydrated the file"
    );
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        reads,
        remote.bytes.reads.load(Ordering::SeqCst),
        "rename reconciliation hydrated the file"
    );
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert!(remote
        .nodes
        .iter()
        .any(|n| n.id == b.id && n.name == "renamed.txt"));
    assert_eq!(
        reads,
        remote.bytes.reads.load(Ordering::SeqCst),
        "rename hydrated a placeholder"
    );
    let mut state = remote.state();
    remote.update(&b, "server-renamed.txt", Some(&folder.id), None, &mut state)?;
    run(&config, &mut store, &mut remote, &mut scanner)?;
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert!(root.join("server-renamed.txt").exists());
    // Local deletion moves an entire directory to the JMAP trash.
    std::fs::remove_dir_all(root.join("nested"))?;
    run(&config, &mut store, &mut remote, &mut scanner)?;
    let trash = remote
        .nodes
        .iter()
        .find(|n| n.role.as_deref() == Some("trash"))
        .unwrap()
        .clone();
    assert_ne!(
        remote
            .nodes
            .iter()
            .find(|n| n.id == nested.id)
            .unwrap()
            .parent_id,
        Some(folder.id.clone())
    );
    assert!(
        remote.nodes.iter().any(|n| n.id == a.id),
        "server trash destroyed the blob"
    );
    let online = remote
        .nodes
        .iter()
        .find(|n| n.name == "trash.txt")
        .unwrap()
        .clone();
    let mut state = remote.state();
    remote.update(&online, &online.name, Some(&trash.id), None, &mut state)?;
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert!(!root.join("trash.txt").exists());
    assert!(check_detach(&config).is_err());
    // Explorer pins the root and descendants. The provider hydrates them.
    {
        let file = Protected::open(&root, false)?;
        let lease = Lease::new(&file)?;
        unsafe {
            CfSetPinState(
                lease.raw(),
                CF_PIN_STATE_PINNED,
                CF_SET_PIN_FLAG_RECURSE,
                None,
            )?;
        }
    }
    scanner.cloud_availability()?;
    check_detach(&config)?;
    std::fs::write(root.join("unseen.txt"), b"edited just before detaching")?;
    drop(scanner);
    drop(store);
    detach(&config).context("Materialize and detach")?;
    assert!(!is_owned(&root.join("server-renamed.txt"))?);
    config.files_on_demand = false;
    assert!(
        !root.join(crate::filesystem::PRIVATE_DIR).exists(),
        "detachment recreated root-local state"
    );
    let mut store = crate::store::Store::open(&config)?;
    let mut scanner = crate::scanner::LocalScanner::new(&root, 900);
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(report.uploaded, 1, "detachment lost the committed baseline");
    assert_eq!(report.downloaded, 0);
    let edited = remote
        .nodes
        .iter()
        .find(|n| n.name == "unseen.txt" && n.parent_id.as_deref() == Some(&folder.id))
        .unwrap();
    assert_eq!(
        remote.bytes.blobs.lock().unwrap()[edited.blob_id.as_ref().unwrap()],
        b"edited just before detaching"
    );
    std::fs::write(root.join("local.txt"), b"changed during full sync")?;
    assert_eq!(
        run(&config, &mut store, &mut remote, &mut scanner)?.uploaded,
        1
    );
    drop(scanner);
    drop(store);
    config.files_on_demand = true;
    let mut store = crate::store::Store::open(&config)?;
    let mut scanner = crate::scanner::LocalScanner::new(&root, 900);
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        report.uploaded + report.downloaded,
        0,
        "re-enabling cloud mode used a stale database"
    );
    {
        let file = Protected::open(&root, false)?;
        let lease = Lease::new(&file)?;
        unsafe {
            CfSetPinState(
                lease.raw(),
                CF_PIN_STATE_UNSPECIFIED,
                CF_SET_PIN_FLAG_RECURSE,
                None,
            )?;
        }
    }
    remote.seed(
        "after-restart.txt",
        Some(&folder.id),
        Some("download after restarting"),
    );
    run(&config, &mut store, &mut remote, &mut scanner)?;
    assert!(check_uninstall().is_err());
    let mut overlap = config.clone();
    overlap.local_folder = root.join("overlap");
    std::fs::create_dir(&overlap.local_folder)?;
    assert!(validate_root(&overlap).is_err());
    std::fs::remove_dir(&overlap.local_folder)?;
    drop(scanner);
    assert!(
        std::fs::read(root.join("after-restart.txt")).is_err(),
        "an online-only file should require a running provider"
    );
    let mut scanner = crate::scanner::LocalScanner::new(&root, 900);
    let report = run(&config, &mut store, &mut remote, &mut scanner)?;
    assert_eq!(
        report.uploaded + report.downloaded,
        0,
        "restart lost the placeholder baseline"
    );
    assert_eq!(
        std::fs::read(root.join("after-restart.txt"))?,
        b"download after restarting"
    );
    drop(scanner);
    drop(store);
    detach(&config)?;
    Ok(())
}
#[test]
#[ignore = "Registers a temporary native Windows Cloud Files root; run explicitly on NTFS"]
fn native_placeholder_hydration_and_eviction() -> Result<()> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    let temp = tempfile::tempdir_in(fixtures)?;
    let root = temp.path().join("CloudFiles");
    std::fs::create_dir(&root)?;
    let config = Config {
        local_folder: root.clone(),
        username: "test@example.org".into(),
        files_on_demand: true,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.clone());
    let bytes = Arc::new(Bytes {
        data: (0..150_123).map(|i| (i % 251) as u8).collect(),
        requests: AtomicUsize::new(0),
        fail_after: AtomicUsize::new(0),
        offsets: Mutex::new(Vec::new()),
    });
    let provider = Provider::connect(&config, bytes.clone())?;
    let node = Node {
        id: "test-node".into(),
        name: "payload.bin".into(),
        parent_id: None,
        blob_id: Some("test-blob".into()),
        kind: Kind::File,
        size: bytes.data.len() as u64,
        role: None,
    };
    let local = provider.publish("payload.bin", &node, None)?;
    let target = root.join("payload.bin");
    assert!(is_owned(&target)?);
    assert_eq!(committed_entry(&target)?, Some(local.clone()));
    assert_eq!(std::fs::metadata(&target)?.len(), node.size);
    assert_eq!(
        bytes.requests.load(Ordering::SeqCst),
        0,
        "metadata access hydrated a placeholder"
    );
    assert_eq!(
        std::fs::read(&target).context("Read initially online-only payload")?,
        bytes.data
    );
    assert!(bytes.requests.load(Ordering::SeqCst) > 0);
    assert_eq!(
        committed_entry(&target)?,
        Some(local.clone()),
        "hydration changed the baseline"
    );
    {
        let file = Protected::open(&target, true)?;
        let lease = Lease::new(&file)?;
        unsafe {
            CfSetPinState(
                lease.raw(),
                CF_PIN_STATE_UNPINNED,
                CF_SET_PIN_FLAG_NONE,
                None,
            )?;
            CfDehydratePlaceholder(lease.raw(), 0, CF_EOF, CF_DEHYDRATE_FLAG_NONE, None)?;
        }
    }
    assert_eq!(committed_entry(&target)?, Some(local));
    assert_eq!(placeholder(&target)?.unwrap().on_disk, 0);
    // Nonzero range requests work without a normal data-stream open.
    {
        let file = metadata_exclusive(&target)?;
        unsafe {
            CfHydratePlaceholder(handle(&file), 8192, 4096, CF_HYDRATE_FLAG_NONE, None)?;
        }
        assert!(placeholder_handle(&file)?.unwrap().on_disk > 0);
    }
    assert_eq!(
        std::fs::read(&target).context("Read payload after nonzero range hydration")?,
        bytes.data
    );
    // Fail a separate request after delivering aligned pages. Retrying the
    // same immutable blob must request only its unhydrated remainder.
    let interrupted_node = Node {
        id: "interrupted".into(),
        name: "interrupted.bin".into(),
        ..node.clone()
    };
    provider.publish("interrupted.bin", &interrupted_node, None)?;
    let interrupted = root.join("interrupted.bin");
    {
        let file = metadata_exclusive(&interrupted)?;
        bytes.fail_after.store(10000, Ordering::SeqCst);
        assert!(unsafe {
            CfHydratePlaceholder(handle(&file), 0, CF_EOF, CF_HYDRATE_FLAG_NONE, None)
        }
        .is_err());
        let partial = placeholder_handle(&file)?.unwrap().on_disk;
        assert!(
            partial > 0 && partial < node.size as i64,
            "completed ranges were discarded: {partial}"
        );
        let offset_count = bytes.offsets.lock().unwrap().len();
        bytes.fail_after.store(0, Ordering::SeqCst);
        unsafe {
            CfHydratePlaceholder(handle(&file), 0, CF_EOF, CF_HYDRATE_FLAG_NONE, None)?;
        }
        assert!(
            bytes.offsets.lock().unwrap()[offset_count..]
                .iter()
                .any(|offset| *offset > 0),
            "retry restarted an already persisted prefix"
        );
    }
    assert_eq!(std::fs::read(&interrupted)?, bytes.data);
    // A local edit must lose the committed marker and may not be evicted.
    std::fs::write(&target, b"local changes")?;
    assert!(committed_entry(&target)?.is_none());
    let file = Protected::open(&target, true)?;
    let lease = Lease::new(&file)?;
    assert!(unsafe {
        CfDehydratePlaceholder(lease.raw(), 0, CF_EOF, CF_DEHYDRATE_FLAG_NONE, None)
    }
    .is_err());
    drop(lease);
    drop(file);
    assert_eq!(std::fs::read(&target)?, b"local changes");
    // Convert a regular uploaded file only when its bytes still match the
    // committed snapshot, under an exclusive protected handle.
    let uploaded = root.join("uploaded.txt");
    std::fs::write(&uploaded, b"uploaded version")?;
    let local_upload = crate::filesystem::hash_file(&uploaded, u64::MAX)?;
    let upload_node = Node {
        id: "uploaded".into(),
        name: "uploaded.txt".into(),
        parent_id: None,
        blob_id: Some("upload-blob".into()),
        kind: Kind::File,
        size: local_upload.size,
        role: None,
    };
    provider.publish("uploaded.txt", &upload_node, Some(&local_upload))?;
    assert_eq!(committed_entry(&uploaded)?, Some(local_upload.clone()));
    std::fs::write(&uploaded, b"new local edit!!")?;
    assert!(provider
        .publish("uploaded.txt", &upload_node, Some(&local_upload))
        .is_err());
    assert!(committed_entry(&uploaded)?.is_none());
    // Fully populated directory placeholders never require enumeration I/O.
    let directory = Node {
        id: "dir".into(),
        name: "nested".into(),
        parent_id: None,
        blob_id: None,
        kind: Kind::Directory,
        size: 0,
        role: None,
    };
    provider.publish("nested", &directory, None)?;
    assert!(is_owned(&root.join("nested"))?);
    let mut child = node.clone();
    child.name = "child.bin".into();
    provider.publish("nested/child.bin", &child, None)?;
    assert!(crate::filesystem::checked_path(&root, "nested/child.bin").is_ok());
    let mut scanner = crate::scanner::LocalScanner::new(&root, 900);
    let before = bytes.requests.load(Ordering::SeqCst);
    scanner.scan(u64::MAX)?;
    assert_eq!(
        before,
        bytes.requests.load(Ordering::SeqCst),
        "scanner hydrated a committed child"
    );
    assert!(
        check_detach(&config).is_err(),
        "a partial root must not be detached"
    );
    drop(provider);
    Ok(())
}

#[test]
fn requested_ranges_are_clipped_at_the_logical_end_of_file() {
    let mut request = Request {
        connection: CF_CONNECTION_KEY::default(),
        transfer: 0,
        request: 0,
        offset: 147456,
        length: 4096,
    };
    normalize_range(&mut request, 150123).unwrap();
    assert_eq!(request.length, 2667);
    request.offset = 8192;
    request.length = CF_EOF;
    normalize_range(&mut request, 150123).unwrap();
    assert_eq!(request.length, 141931);
    request.length = i64::MAX;
    normalize_range(&mut request, 150123).unwrap();
    assert_eq!(request.length, 141931);
    request.offset = -1;
    assert!(normalize_range(&mut request, 150123).is_err());
    request.offset = 150124;
    assert!(normalize_range(&mut request, 150123).is_err());
    request.offset = 0;
    request.length = -2;
    assert!(normalize_range(&mut request, 150123).is_err());
}

#[test]
#[ignore = "Exercises a real Windows oplock break during placeholder conversion"]
fn protected_snapshot_remains_valid_during_a_conflicting_open() -> Result<()> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/cloud-tests");
    std::fs::create_dir_all(&fixtures)?;
    let temp = tempfile::tempdir_in(fixtures)?;
    let root = temp.path().join("Oplock");
    std::fs::create_dir(&root)?;
    let config = Config {
        local_folder: root.clone(),
        username: "test@example.org".into(),
        files_on_demand: true,
        ..Default::default()
    };
    let _cleanup = Cleanup(config.clone());
    let bytes = Arc::new(MemoryBytes::default());
    let _provider = Provider::connect(&config, bytes)?;
    let path = root.join("uploaded.txt");
    std::fs::write(&path, b"committed contents")?;
    let local = crate::filesystem::hash_file(&path, u64::MAX)?;
    let node = Node {
        id: "uploaded".into(),
        name: "uploaded.txt".into(),
        parent_id: None,
        blob_id: Some("immutable".into()),
        role: None,
        kind: Kind::File,
        size: local.size,
    };
    let identity =
        Identity::new(&super::super::root_key(&config)?, &node, Some(&local)).encode()?;
    let file = Protected::open(&path, true)?;
    let lease = Lease::new(&file)?;
    let opener_path = path.clone();
    let opener = std::thread::spawn(move || OpenOptions::new().write(true).open(opener_path));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if !unsafe { CfReferenceProtectedHandle(file.0) } {
            break;
        }
        unsafe {
            CfReleaseProtectedHandle(file.0);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "conflicting open did not request an oplock break"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    lease.verify(&local)?;
    unsafe {
        CfConvertToPlaceholder(
            lease.raw(),
            Some(identity.as_ptr().cast()),
            identity.len() as u32,
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            None,
            None,
        )
        .context("Convert while oplock break is pending")?;
    }
    drop(lease);
    drop(file);
    drop(opener.join().unwrap()?);
    assert_eq!(committed_entry(&path)?, Some(local));
    assert_eq!(std::fs::read(&path)?, b"committed contents");
    Ok(())
}
