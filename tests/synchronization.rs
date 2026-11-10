//! Stateful integration tests: real filesystem + SQLite, simulated immutable
//! JMAP blobs and global ifInState. No production account is ever contacted.
use anyhow::{bail, Result};
use filenode::{config::Config, engine, filesystem, jmap::Remote, model::*, store::Store};
use std::{collections::HashMap, fs, path::Path, sync::atomic::AtomicBool};

fn multiple_folder_config(home: &Path) -> Config {
    let selected = [home.join("Desktop"), home.join("Documents").join("Büro")];
    for path in &selected {
        fs::create_dir_all(path).unwrap();
    }
    Config {
        username: "test@example.org".into(),
        file_settle_seconds: 0,
        sync_folders: Some(
            selected
                .into_iter()
                .map(|local_folder| filenode::config::SyncFolder {
                    remote_path: Config::derive_remote_path(&local_folder, home).unwrap(),
                    local_folder,
                    files_on_demand: None,
                })
                .collect(),
        ),
        ..Config::default()
    }
}

#[test]
fn folder_failures_reuse_the_account_session_and_obey_retry_deadlines() {
    use filenode::group::{SyncGroup, SyncSession};
    let home = tempfile::tempdir().unwrap();
    let mut config = multiple_folder_config(home.path());
    config.sync_folders.as_mut().unwrap().truncate(1);
    let root = config.sync_folders.as_ref().unwrap()[0]
        .local_folder
        .clone();
    let mut group = SyncGroup::new(&config).unwrap();
    fs::remove_dir(&root).unwrap();
    let mut session = SyncSession::default();
    let mut connections = 0;
    let cancel = AtomicBool::new(false);
    let mut connect = || {
        connections += 1;
        Ok(Server::default())
    };
    assert_eq!(
        session
            .poll(&mut group, &mut connect, &cancel, &mut |_| {})
            .unwrap()
            .unwrap()
            .failed_folders,
        1
    );
    for _ in 0..100 {
        assert!(session
            .poll(&mut group, &mut connect, &cancel, &mut |_| {})
            .unwrap()
            .is_none());
    }
    // An explicit check can retry now, but it still uses the same transport.
    group.invalidate();
    assert_eq!(
        session
            .poll(&mut group, &mut connect, &cancel, &mut |_| {})
            .unwrap()
            .unwrap()
            .failed_folders,
        1
    );
    // Even after an actual session reset, the folder deadline still applies.
    session.reset();
    assert!(session
        .poll(&mut group, &mut connect, &cancel, &mut |_| {})
        .unwrap()
        .is_none());
    fs::create_dir(&root).unwrap();
    fs::write(root.join("recovered.txt"), "available").unwrap();
    group.invalidate();
    let report = session
        .poll(&mut group, &mut connect, &cancel, &mut |_| {})
        .unwrap()
        .unwrap();
    assert_eq!(report.failed_folders, 0);
    assert_eq!(report.uploaded, 1);
    assert_eq!(connections, 2);
}

#[test]
fn multiple_folders_use_leaf_names_and_stop_without_deleting() {
    use filenode::group::SyncGroup;
    let home = tempfile::tempdir().unwrap();
    let mut config = multiple_folder_config(home.path());
    let folders = config.sync_folders.as_ref().unwrap().clone();
    assert_eq!(folders[0].remote_path, "Desktop");
    assert_eq!(folders[1].remote_path, "Büro");
    fs::write(folders[0].local_folder.join("same.txt"), "desktop original").unwrap();
    fs::write(folders[1].local_folder.join("same.txt"), "office original").unwrap();
    let mut server = Server::default();
    let mut state = server.state();
    let documents = server
        .create("Documents", None, Kind::Directory, None, 0, &mut state)
        .unwrap();
    let sibling = server
        .create(
            "Unselected",
            Some(&documents.id),
            Kind::Directory,
            None,
            0,
            &mut state,
        )
        .unwrap();
    let mut group = SyncGroup::new(&config).unwrap();
    assert_eq!(
        group
            .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .uploaded,
        2
    );
    let snapshot = server.snapshot(None).unwrap();
    let desktop = engine::find_folder_path(&snapshot, "Desktop")
        .unwrap()
        .unwrap();
    let office = engine::find_folder_path(&snapshot, "Büro")
        .unwrap()
        .unwrap();
    assert!(server
        .nodes
        .iter()
        .any(|node| node.id == sibling.id && node.parent_id.as_ref() == Some(&documents.id)));
    assert!(!server
        .nodes
        .iter()
        .any(|node| node.name == "Büro" && node.parent_id.as_ref() == Some(&documents.id)));
    let desktop_file = server
        .nodes
        .iter()
        .find(|node| node.parent_id.as_ref() == Some(&desktop) && node.name == "same.txt")
        .unwrap()
        .id
        .clone();
    let office_file = server
        .nodes
        .iter()
        .find(|node| node.parent_id.as_ref() == Some(&office) && node.name == "same.txt")
        .unwrap()
        .id
        .clone();
    assert_ne!(desktop_file, office_file);
    drop(group);
    config.sync_folders.as_mut().unwrap().remove(0);
    fs::write(
        folders[0].local_folder.join("same.txt"),
        "unsynchronized local edit",
    )
    .unwrap();
    fs::write(folders[1].local_folder.join("same.txt"), "office update").unwrap();
    let mut group = SyncGroup::new(&config).unwrap();
    assert_eq!(
        group
            .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .failed_folders,
        0
    );
    let node = server
        .nodes
        .iter()
        .find(|node| node.id == desktop_file)
        .unwrap();
    assert_eq!(
        server.blobs[node.blob_id.as_ref().unwrap()],
        b"desktop original"
    );
    assert_eq!(
        fs::read(folders[0].local_folder.join("same.txt")).unwrap(),
        b"unsynchronized local edit"
    );
    let node = server
        .nodes
        .iter()
        .find(|node| node.id == office_file)
        .unwrap();
    assert_eq!(
        server.blobs[node.blob_id.as_ref().unwrap()],
        b"office update"
    );
}

#[test]
fn an_unavailable_folder_does_not_recreate_local_roots_or_block_healthy_folders() {
    use filenode::{config::SyncFolder, group::SyncGroup};
    let home = tempfile::tempdir().unwrap();
    let mut config = multiple_folder_config(home.path());
    config.sync_folders.as_mut().unwrap().truncate(1);
    let missing = home.path().join("Offline");
    config.sync_folders.as_mut().unwrap().insert(
        0,
        SyncFolder {
            local_folder: missing.clone(),
            remote_path: "Offline".into(),
            files_on_demand: None,
        },
    );
    fs::write(home.path().join("Desktop/healthy.txt"), "available").unwrap();
    let mut server = Server::default();
    let mut group = SyncGroup::new(&config).unwrap();
    let report = group
        .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
        .unwrap();
    assert_eq!(report.failed_folders, 1);
    assert_eq!(report.uploaded, 1);
    assert!(!missing.exists());
    assert!(!server.nodes.iter().any(|node| node.name == "Offline"));
    fs::create_dir(&missing).unwrap();
    fs::write(missing.join("returned.txt"), "back online").unwrap();
    group.invalidate();
    let report = group
        .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
        .unwrap();
    assert_eq!(report.failed_folders, 0);
    assert_eq!(report.uploaded, 1);
}

#[test]
fn multiple_roots_reject_overlaps_case_collisions_and_unsafe_remote_paths() {
    let home = tempfile::tempdir().unwrap();
    let mut config = multiple_folder_config(home.path());
    let original = config.sync_folders.as_ref().unwrap().clone();
    config.sync_folders.as_mut().unwrap()[1].local_folder = home.path().join("Desktop/nested");
    fs::create_dir_all(home.path().join("Desktop/nested")).unwrap();
    assert!(config.validate().is_err());
    config.sync_folders = Some(original.clone());
    config.sync_folders.as_mut().unwrap()[1].remote_path = "desktop".into();
    assert!(config.validate().is_err());
    config.sync_folders = Some(original);
    config.sync_folders.as_mut().unwrap()[1].remote_path = "Desktop/subfolder".into();
    assert!(config.validate().is_err());
    for invalid in [
        "../Desktop",
        "/Desktop",
        "Documents//Büro",
        "Documents/../Büro",
        "Documents\\Büro",
        ".filenode/history",
    ] {
        assert!(Config::validate_remote_path(invalid).is_err(), "{invalid}");
    }
    // Nested selections use only their own name; equal leaf names cannot bind
    // two independent local trees to the same remote directory.
    let taxes = home.path().join("Dokumente/Steuern");
    fs::create_dir_all(&taxes).unwrap();
    assert_eq!(
        Config::derive_remote_path(&taxes, home.path()).unwrap(),
        "Steuern"
    );
    let duplicate = home.path().join("Archive/Desktop");
    fs::create_dir_all(&duplicate).unwrap();
    let mut collisions = multiple_folder_config(home.path());
    collisions
        .sync_folders
        .as_mut()
        .unwrap()
        .push(filenode::config::SyncFolder {
            remote_path: Config::derive_remote_path(&duplicate, home.path()).unwrap(),
            local_folder: duplicate,
            files_on_demand: None,
        });
    assert!(collisions.validate().is_err());
    assert!(Config::derive_remote_path(home.path(), home.path()).is_err());
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::derive_remote_path(outside.path(), home.path()).unwrap(),
        outside.path().file_name().unwrap().to_str().unwrap()
    );
    assert!(!home
        .path()
        .join("Desktop")
        .join(filesystem::PRIVATE_DIR)
        .exists());
}

#[test]
fn multi_folder_power_pause_blocks_all_scans_and_state_creation() {
    use filenode::{
        group::SyncGroup,
        runtime::{Policy, PowerState, Scope},
    };
    use std::sync::Arc;
    let home = tempfile::tempdir().unwrap();
    let config = multiple_folder_config(home.path());
    fs::write(home.path().join("Desktop/a.txt"), "one").unwrap();
    let power = Arc::new(PowerState::default());
    let _scope = Scope::enter(Policy::new(
        &config,
        Arc::new(AtomicBool::new(false)),
        power.clone(),
    ));
    let mut group = SyncGroup::new(&config).unwrap();
    let mut server = Server::default();
    power.set(true, false);
    assert!(group
        .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
        .is_err());
    assert_eq!(server.writes, 0);
    for folder in config.sync_folders.as_ref().unwrap() {
        assert!(!folder.local_folder.join(filesystem::PRIVATE_DIR).exists());
    }
    power.set(false, false);
    assert_eq!(
        group
            .poll(&mut server, &AtomicBool::new(false), &mut |_| {})
            .unwrap()
            .uploaded,
        1
    );
}

#[test]
fn locked_file_is_deferred_without_deletion_while_other_files_sync() {
    let mut f = Fixture::new();
    f.local("busy.txt", "original");
    f.run().unwrap();
    let handle = fs::File::options()
        .read(true)
        .write(true)
        .open(f.folder.path().join("busy.txt"))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&handle).unwrap();
    f.server.edit("busy.txt", "remote edit");
    f.local("available.txt", "new file");
    let report = f.run().unwrap();
    assert!(report.deferred > 0);
    assert_eq!(report.recycled, 0);
    assert_eq!(report.uploaded, 1);
    drop(handle);
    // Windows byte-range locks also block reads through another handle.
    assert_eq!(f.read("busy.txt"), "original");
    f.run().unwrap();
    assert_eq!(f.read("busy.txt"), "remote edit");
    assert!(f.history_contains(b"original"));
}

#[test]
fn changing_file_must_settle_before_upload() {
    use filenode::scanner::LocalScanner;
    use std::time::Duration;
    let mut f = Fixture::new();
    f.config.file_settle_seconds = 1;
    f.local("growing.txt", "first chunk");
    let mut store = Store::open(&f.config).unwrap();
    let mut scanner = LocalScanner::new(f.folder.path(), 900);
    let mut run = |f: &mut Fixture| {
        engine::sync_with_scanner(
            &f.config,
            &mut store,
            &mut f.server,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut scanner,
        )
        .unwrap()
    };
    assert_eq!(run(&mut f).deferred, 1);
    f.local("growing.txt", "complete document");
    assert_eq!(run(&mut f).deferred, 1);
    std::thread::sleep(Duration::from_millis(550));
    assert_eq!(run(&mut f).deferred, 1);
    assert!(!f.server.nodes.iter().any(|node| node.name == "growing.txt"));
    std::thread::sleep(Duration::from_millis(550));
    assert_eq!(run(&mut f).uploaded, 1);
    let node = f.server.node("growing.txt");
    assert_eq!(
        f.server.blobs[node.blob_id.as_ref().unwrap()],
        b"complete document"
    );
}

#[test]
fn power_pause_prevents_scan_and_remote_mutations_until_resumed() {
    use filenode::runtime::{Policy, PowerState, Scope};
    use std::sync::Arc;
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    let power = Arc::new(PowerState::default());
    let policy = Policy::new(&f.config, Arc::new(AtomicBool::new(false)), power.clone());
    let _scope = Scope::enter(policy);
    power.set(true, false);
    assert!(f.run().is_err());
    assert_eq!(f.server.writes, 0);
    power.set(false, true);
    assert!(f.run().is_err());
    assert_eq!(f.server.writes, 0);
    power.set(false, false);
    assert_eq!(f.run().unwrap().uploaded, 1);
}

#[test]
fn idle_sync_reuses_hashes_and_does_not_rewrite_unchanged_baselines() {
    let mut fixture = Fixture::new();
    fixture.local("a.txt", "one");
    let mut store = Store::open(&fixture.config).unwrap();
    let mut scanner = filenode::scanner::LocalScanner::new(fixture.folder.path(), 900);
    for _ in 0..2 {
        engine::sync_with_scanner(
            &fixture.config,
            &mut store,
            &mut fixture.server,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut scanner,
        )
        .unwrap();
    }
    let writes = store.db.total_changes();
    let report = engine::sync_with_scanner(
        &fixture.config,
        &mut store,
        &mut fixture.server,
        &AtomicBool::new(false),
        &mut |_| {},
        &mut scanner,
    )
    .unwrap();
    assert_eq!(store.db.total_changes(), writes);
    assert_eq!(report.uploaded + report.downloaded, 0);
    if scanner.watching() {
        assert_eq!(scanner.hashed_files(), 0);
    }
    fixture.local("a.txt", "two");
    std::thread::sleep(std::time::Duration::from_millis(150));
    engine::sync_with_scanner(
        &fixture.config,
        &mut store,
        &mut fixture.server,
        &AtomicBool::new(false),
        &mut |_| {},
        &mut scanner,
    )
    .unwrap();
    let node = fixture.server.node("a.txt");
    assert_eq!(fixture.server.blobs[node.blob_id.as_ref().unwrap()], b"two");
}

#[derive(Default)]
struct Server {
    limits: filenode::jmap::UploadLimits,
    uploader: Option<std::sync::Arc<dyn filenode::jmap::Uploader>>,
    nodes: Vec<Node>,
    blobs: HashMap<String, Vec<u8>>,
    revision: usize,
    writes: usize,
    fail_snapshot: bool,
    fail_after_create: bool,
    race_on_upload: bool,
}

impl Server {
    fn state(&self) -> String {
        self.revision.to_string()
    }
    fn guard(&self, state: &str) -> Result<()> {
        if self.state() != state {
            bail!("stateMismatch");
        }
        Ok(())
    }
    fn node(&self, name: &str) -> Node {
        self.nodes.iter().find(|n| n.name == name).unwrap().clone()
    }
    fn edit(&mut self, name: &str, content: &str) {
        let blob = uuid::Uuid::new_v4().to_string();
        self.blobs.insert(blob.clone(), content.as_bytes().to_vec());
        let node = self.nodes.iter_mut().find(|n| n.name == name).unwrap();
        node.blob_id = Some(blob);
        node.size = content.len() as u64;
        self.revision += 1;
    }
}

impl Remote for Server {
    fn upload_limits(&self) -> filenode::jmap::UploadLimits {
        self.limits
    }
    fn uploader(&self) -> Option<std::sync::Arc<dyn filenode::jmap::Uploader>> {
        self.uploader.clone()
    }
    fn create_trash(&mut self, parent: Option<&str>, state: &mut String) -> Result<Node> {
        let mut node = self.create(".Trash", parent, Kind::Directory, None, 0, state)?;
        node.role = Some("trash".into());
        self.nodes
            .iter_mut()
            .find(|item| item.id == node.id)
            .unwrap()
            .role = node.role.clone();
        Ok(node)
    }
    fn account_id(&self) -> &str {
        "account"
    }
    fn current_state(&self) -> Result<String> {
        Ok(self.state())
    }
    fn snapshot(&mut self, _previous: Option<&Snapshot>) -> Result<Snapshot> {
        if self.fail_snapshot {
            bail!("Simulated connection failure");
        }
        Ok(Snapshot {
            state: self.state(),
            nodes: self.nodes.clone(),
        })
    }
    fn upload(&mut self, path: &Path, _size: u64) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.blobs.insert(id.clone(), fs::read(path)?);
        if self.race_on_upload {
            self.revision += 1;
        }
        Ok(id)
    }
    fn download(&mut self, node: &Node, path: &Path) -> Result<()> {
        fs::write(path, &self.blobs[node.blob_id.as_ref().unwrap()])?;
        Ok(())
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
        self.guard(state)?;
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
        self.writes += 1;
        *state = self.state();
        if self.fail_after_create && kind == Kind::File {
            self.fail_after_create = false;
            bail!("Connection lost after server commit");
        }
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
        self.guard(state)?;
        let existing = self.nodes.iter_mut().find(|n| n.id == node.id).unwrap();
        existing.name = name.into();
        existing.parent_id = parent.map(str::to_owned);
        if let Some((blob, size)) = blob {
            existing.blob_id = Some(blob.into());
            existing.size = size;
        }
        let updated = existing.clone();
        self.revision += 1;
        self.writes += 1;
        *state = self.state();
        Ok(updated)
    }
}

struct Fixture {
    folder: tempfile::TempDir,
    config: Config,
    server: Server,
}

#[test]
fn oversized_uploads_emit_file_warnings_without_blocking_small_files() {
    let mut f = Fixture::new();
    f.server.limits.max_size_upload = Some(4);
    f.local("small.txt", "abc");
    f.local("large.txt", "abcdef");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = events.clone();
    let _observer =
        filenode::activity::Scope::observe(move |event| sink.lock().unwrap().push(event));
    let report = f.run().unwrap();
    assert_eq!(report.uploaded, 1);
    assert_eq!(report.limited, 1);
    assert!(!f.server.nodes.iter().any(|node| node.name == "large.txt"));
    assert_eq!(f.read("large.txt"), "abcdef");
    assert!(events.lock().unwrap().iter().any(|event| matches!(event, filenode::activity::Event::FileLimit { path, size: 6, limit: 4, .. } if path == "large.txt")));
    f.server.limits.max_size_upload = Some(10);
    assert_eq!(f.run().unwrap().uploaded, 1);
}

#[test]
fn parallel_blobs_obey_server_and_user_limits_then_commit_serially() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Upload {
        active: AtomicUsize,
        peak: AtomicUsize,
        first_pair: (std::sync::Mutex<usize>, std::sync::Condvar),
        policy: Arc<filenode::runtime::Policy>,
    }
    impl filenode::jmap::Uploader for Upload {
        fn policy(&self) -> Arc<filenode::runtime::Policy> {
            self.policy.clone()
        }
        fn send_blob(&self, path: &Path, size: u64) -> Result<String> {
            let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(count, Ordering::SeqCst);
            assert_eq!(fs::read(path)?.len() as u64, size);
            // Staging can take longer than a fixed sleep on a busy runner.
            // Require the first two uploads to overlap with a bounded rendezvous.
            let (arrivals, ready) = &self.first_pair;
            let mut arrivals = arrivals.lock().unwrap();
            *arrivals += 1;
            ready.notify_all();
            let (arrivals, _) = ready
                .wait_timeout_while(arrivals, std::time::Duration::from_secs(10), |count| {
                    *count < 2
                })
                .unwrap();
            assert!(*arrivals >= 2, "Upload workers did not run concurrently");
            drop(arrivals);
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(uuid::Uuid::new_v4().to_string())
        }
    }
    let mut f = Fixture::new();
    f.config.parallel_uploads = 8;
    f.server.limits.max_concurrent_upload = Some(2);
    let upload = Arc::new(Upload {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        first_pair: Default::default(),
        policy: filenode::runtime::Policy::new(
            &f.config,
            Arc::new(AtomicBool::new(false)),
            Arc::new(Default::default()),
        ),
    });
    f.server.uploader = Some(upload.clone());
    for index in 0..5 {
        f.local(&format!("{index}.txt"), "payload");
    }
    assert_eq!(f.run().unwrap().uploaded, 5);
    assert_eq!(upload.peak.load(Ordering::SeqCst), 2);
    assert_eq!(f.run().unwrap().uploaded, 0);
    f.config.parallel_uploads = 1;
    upload.peak.store(0, Ordering::SeqCst);
    f.local("serial.txt", "payload");
    assert_eq!(f.run().unwrap().uploaded, 1);
    assert_eq!(upload.peak.load(Ordering::SeqCst), 0); // Serial Remote::upload path.
}

#[test]
fn exclusions_skip_both_directions_and_retain_baselines_without_deletion() {
    let mut f = Fixture::new();
    f.config.exclude_patterns.clear();
    f.local("local.tmp", "keep remote copy");
    f.local("remote.tmp", "keep local copy");
    f.run().unwrap();
    f.config.exclude_patterns = vec!["*.tmp".into()];
    fs::remove_file(f.folder.path().join("local.tmp")).unwrap();
    f.server.nodes.retain(|node| node.name != "remote.tmp");
    f.server.revision += 1;
    f.local("new.tmp", "do not upload");
    f.local("normal.txt", "upload normally");
    let report = f.run().unwrap();
    assert_eq!(report.uploaded, 1);
    assert_eq!(report.recycled, 0);
    assert_eq!(f.read("remote.tmp"), "keep local copy");
    assert!(f.server.nodes.iter().any(|node| node.name == "local.tmp"));
    assert!(!f.server.nodes.iter().any(|node| node.name == "new.tmp"));
    assert!(Store::open(&f.config)
        .unwrap()
        .baselines()
        .unwrap()
        .contains_key("local.tmp"));
    f.config.exclude_patterns.clear();
    assert_eq!(f.run().unwrap().uploaded, 1);
    assert!(f.server.nodes.iter().any(|node| node.name == "new.tmp"));
}

#[test]
fn excluded_children_prevent_parent_deletion_but_allow_sibling_updates() {
    let mut f = Fixture::new();
    fs::create_dir(f.folder.path().join("Docs")).unwrap();
    f.local("Docs/normal.txt", "original");
    f.local("Docs/private.tmp", "local only");
    f.run().unwrap();
    f.local("Docs/normal.txt", "updated");
    assert_eq!(f.run().unwrap().uploaded, 1);
    assert_eq!(f.read("Docs/private.tmp"), "local only");
    let docs = f.server.node("Docs").id;
    f.server
        .nodes
        .retain(|node| node.id != docs && node.parent_id.as_deref() != Some(&docs));
    f.server.revision += 1;
    let report = f.run().unwrap();
    assert!(report.blocked > 0);
    assert_eq!(f.read("Docs/private.tmp"), "local only");
    assert_eq!(f.read("Docs/normal.txt"), "updated");
}
impl Fixture {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        let config = Config {
            username: "test@example.org".into(),
            local_folder: folder.path().to_owned(),
            propagate_deletes: true,
            file_settle_seconds: 0,
            ..Config::default()
        };
        Self {
            folder,
            config,
            server: Server::default(),
        }
    }
    fn run(&mut self) -> Result<engine::Report> {
        let mut store = Store::open(&self.config)?;
        engine::sync(
            &self.config,
            &mut store,
            &mut self.server,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }
    fn local(&self, path: &str, value: &str) {
        fs::write(self.folder.path().join(path), value).unwrap();
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.folder.path().join(path)).unwrap()
    }
    fn history_contains(&self, value: &[u8]) -> bool {
        walkdir::WalkDir::new(self.config.private_directory().unwrap().join("history"))
            .into_iter()
            .filter_map(Result::ok)
            .any(|e| e.file_type().is_file() && fs::read(e.path()).unwrap() == value)
    }
}

#[test]
fn local_folder_name_selects_remote_directory_and_scopes_all_operations() {
    let mut f = Fixture::new();
    let expected_name = f.config.remote_folder_name().unwrap().to_owned();
    fs::create_dir(f.folder.path().join("docs")).unwrap();
    f.local("docs/local.txt", "nested");
    f.local("top.txt", "top-level");
    assert_eq!(f.run().unwrap().uploaded, 3);
    let root = f.server.node(&expected_name);
    assert!(root.parent_id.is_none());
    let top = f.server.node("top.txt");
    assert_eq!(top.parent_id.as_deref(), Some(root.id.as_str()));
    let docs = f.server.node("docs");
    assert_eq!(docs.parent_id.as_deref(), Some(root.id.as_str()));
    assert_eq!(f.server.node("local.txt").parent_id, Some(docs.id.clone()));
    f.server
        .blobs
        .insert("incoming".into(), b"from server".to_vec());
    let mut state = f.server.state();
    f.server
        .create(
            "incoming.txt",
            Some(&root.id),
            Kind::File,
            Some("incoming"),
            11,
            &mut state,
        )
        .unwrap();
    f.server
        .create(
            "unrelated.txt",
            None,
            Kind::File,
            Some("incoming"),
            11,
            &mut state,
        )
        .unwrap();
    assert_eq!(f.run().unwrap().downloaded, 1);
    assert_eq!(f.read("incoming.txt"), "from server");
    assert!(!f.folder.path().join("unrelated.txt").exists());
    let writes = f.server.writes;
    f.run().unwrap();
    assert_eq!(f.server.writes, writes);
    fs::rename(
        f.folder.path().join("top.txt"),
        f.folder.path().join("docs/moved.txt"),
    )
    .unwrap();
    assert_eq!(f.run().unwrap().renamed, 1);
    assert_eq!(f.server.node("moved.txt").id, top.id);
    assert_eq!(f.server.node("moved.txt").parent_id, Some(docs.id));
    f.run().unwrap();
    fs::rename(
        f.folder.path().join("docs/moved.txt"),
        f.folder.path().join("back.txt"),
    )
    .unwrap();
    assert_eq!(f.run().unwrap().renamed, 1);
    assert_eq!(f.server.node("back.txt").id, top.id);
    assert_eq!(f.server.node("back.txt").parent_id, Some(root.id));
}

#[test]
fn desktop_folder_name_is_derived_and_filesystem_roots_are_rejected() {
    let f = Fixture::new();
    let desktop = f.folder.path().join("Desktop");
    fs::create_dir(&desktop).unwrap();
    let mut config = Config {
        local_folder: desktop,
        ..f.config.clone()
    };
    assert_eq!(config.remote_folder_name().unwrap(), "Desktop");
    config.validate().unwrap();
    config.local_folder.push("");
    assert_eq!(config.remote_folder_name().unwrap(), "Desktop");
    #[cfg(windows)]
    {
        config.local_folder = std::path::PathBuf::from("C:/");
    }
    #[cfg(not(windows))]
    {
        config.local_folder = std::path::PathBuf::from("/");
    }
    assert!(config.validate().is_err());
}

#[test]
fn legacy_named_remote_binding_is_not_silently_reassigned() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    {
        let store = Store::open(&f.config).unwrap();
        let identity = store.get_meta::<String>("identity").unwrap().unwrap();
        let (prefix, _) = identity.rsplit_once('|').unwrap();
        store
            .set_meta("identity", &format!("{prefix}|Legacy remote folder"))
            .unwrap();
    }
    let writes = f.server.writes;
    assert!(f
        .run()
        .unwrap_err()
        .to_string()
        .contains("different server, account or remote folder"));
    assert_eq!(f.server.writes, writes);
    assert_eq!(f.read("a.txt"), "one");
}

#[test]
fn initial_upload_idempotence_and_remote_download() {
    let mut f = Fixture::new();
    fs::create_dir(f.folder.path().join("docs")).unwrap();
    f.local("docs/a.txt", "one");
    assert_eq!(f.run().unwrap().uploaded, 2);
    let writes = f.server.writes;
    f.run().unwrap();
    assert_eq!(f.server.writes, writes);
    f.server.edit("a.txt", "two");
    f.run().unwrap();
    assert_eq!(f.read("docs/a.txt"), "two");
    assert!(f.history_contains(b"one"));
}

#[test]
fn concurrent_edits_preserve_and_upload_both_versions() {
    let mut f = Fixture::new();
    f.local("a.txt", "original");
    f.run().unwrap();
    f.local("a.txt", "my edit");
    f.server.edit("a.txt", "their edit");
    assert_eq!(f.run().unwrap().conflicts, 1);
    assert_eq!(f.read("a.txt"), "their edit");
    let conflict = fs::read_dir(f.folder.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains("conflict"))
        .unwrap();
    assert_eq!(fs::read_to_string(conflict.path()).unwrap(), "my edit");
    f.run().unwrap();
    assert_eq!(
        f.server
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::File)
            .count(),
        2
    );
}

#[test]
fn ambiguous_create_is_reconciled_without_duplicate() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.server.fail_after_create = true;
    assert!(f.run().is_err());
    f.run().unwrap();
    assert_eq!(
        f.server.nodes.iter().filter(|n| n.name == "a.txt").count(),
        1
    );
}

#[test]
fn overwrite_keeps_remote_recovery_copy() {
    let mut f = Fixture::new();
    f.local("a.txt", "old");
    f.run().unwrap();
    let blob = f.server.node("a.txt").blob_id.unwrap();
    f.local("a.txt", "new");
    f.run().unwrap();
    assert!(f
        .server
        .nodes
        .iter()
        .any(|n| n.name != "a.txt" && n.blob_id.as_ref() == Some(&blob)));
    assert_ne!(f.server.node("a.txt").blob_id.as_ref(), Some(&blob));
}

#[test]
fn local_delete_uses_the_native_server_trash_without_a_download_loop_and_restores() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    let id = f.server.node("a.txt").id;
    fs::remove_file(f.folder.path().join("a.txt")).unwrap();
    assert_eq!(f.run().unwrap().recycled, 1);
    let trash = f
        .server
        .nodes
        .iter()
        .find(|node| node.role.as_deref() == Some("trash"))
        .unwrap()
        .id
        .clone();
    let moved = f.server.nodes.iter().find(|node| node.id == id).unwrap();
    let batch = f
        .server
        .nodes
        .iter()
        .find(|node| Some(&node.id) == moved.parent_id.as_ref())
        .unwrap();
    assert_eq!(batch.parent_id.as_ref(), Some(&trash));
    assert!(batch.name.ends_with(&format!(
        "~{}",
        f.server.node(f.config.remote_folder_name().unwrap()).id
    )));
    assert_eq!(moved.name, "a.txt");
    f.run().unwrap();
    assert!(!f.folder.path().join("a.txt").exists());
    let root = f.server.node(f.config.remote_folder_name().unwrap()).id;
    let node = f
        .server
        .nodes
        .iter_mut()
        .find(|node| node.id == id)
        .unwrap();
    node.parent_id = Some(root);
    f.server.revision += 1;
    assert_eq!(f.run().unwrap().downloaded, 1);
    assert_eq!(f.read("a.txt"), "one");
}

#[test]
fn deleting_a_directory_preserves_its_tree_in_one_bulwark_compatible_trash_batch() {
    let mut f = Fixture::new();
    fs::create_dir(f.folder.path().join("docs")).unwrap();
    f.local("docs/a.txt", "one");
    f.run().unwrap();
    let directory = f.server.node("docs").id;
    let file = f.server.node("a.txt").id;
    fs::remove_dir_all(f.folder.path().join("docs")).unwrap();
    assert_eq!(f.run().unwrap().recycled, 1);
    let moved = f
        .server
        .nodes
        .iter()
        .find(|node| node.id == directory)
        .unwrap();
    let batch = f
        .server
        .nodes
        .iter()
        .find(|node| Some(&node.id) == moved.parent_id.as_ref())
        .unwrap();
    let trash = f
        .server
        .nodes
        .iter()
        .find(|node| Some(&node.id) == batch.parent_id.as_ref())
        .unwrap();
    assert_eq!(trash.name, ".Trash");
    assert!(batch.name.contains('~'));
    assert_eq!(
        f.server
            .nodes
            .iter()
            .find(|node| node.id == file)
            .unwrap()
            .parent_id
            .as_deref(),
        Some(directory.as_str())
    );
    assert_eq!(f.run().unwrap().downloaded, 0);
}

#[test]
fn deleting_368_local_entries_moves_one_intact_server_tree_and_other_files_keep_syncing() {
    let mut f = Fixture::new();
    fs::create_dir(f.folder.path().join("Books")).unwrap();
    for section in 1..8 {
        fs::create_dir(f.folder.path().join(format!("Books/Section{section}"))).unwrap();
    }
    let paths: Vec<_> = (0..360)
        .map(|index| {
            let section = index / 45;
            if section == 0 {
                format!("Books/book{index}.txt")
            } else {
                format!("Books/Section{section}/book{index}.txt")
            }
        })
        .collect();
    for (index, path) in paths.iter().enumerate() {
        f.local(path, &format!("Book {index}"));
    }
    assert_eq!(f.run().unwrap().uploaded, 368);
    let root = f.server.node(f.config.remote_folder_name().unwrap()).id;
    let books = f.server.node("Books").id;
    let originals = f.server.nodes.clone();
    fs::remove_dir_all(f.folder.path().join("Books")).unwrap();
    f.local("new.txt", "Unrelated new file");

    let report = f.run().unwrap();
    assert_eq!(report.recycled, 1);
    assert_eq!(report.uploaded, 1);
    assert_eq!(report.blocked, 0);
    let moved = f.server.nodes.iter().find(|node| node.id == books).unwrap();
    let batch = f
        .server
        .nodes
        .iter()
        .find(|node| Some(&node.id) == moved.parent_id.as_ref())
        .unwrap();
    let trash = f
        .server
        .nodes
        .iter()
        .find(|node| Some(&node.id) == batch.parent_id.as_ref())
        .unwrap();
    assert_eq!(trash.role.as_deref(), Some("trash"));
    assert!(batch.name.ends_with(&format!("~{root}")));
    // Every server object and blob survives; only the deleted tree's root moves.
    for before in &originals {
        let after = f
            .server
            .nodes
            .iter()
            .find(|node| node.id == before.id)
            .unwrap();
        assert_eq!(after.name, before.name);
        assert_eq!(after.blob_id, before.blob_id);
        if before.id != books {
            assert_eq!(after.parent_id, before.parent_id);
        }
    }
    let store = Store::open(&f.config).unwrap();
    assert_eq!(store.baselines().unwrap().len(), 1);
    drop(store);
    let writes = f.server.writes;
    let report = f.run().unwrap();
    assert_eq!(report.recycled + report.uploaded + report.downloaded, 0);
    assert_eq!(f.server.writes, writes);
    assert!(!f.folder.path().join("Books").exists());

    // A normal server-side restore downloads the whole tree again.
    f.server
        .nodes
        .iter_mut()
        .find(|node| node.id == books)
        .unwrap()
        .parent_id = Some(root);
    f.server.revision += 1;
    assert_eq!(f.run().unwrap().downloaded, 368);
    for (index, path) in paths.iter().enumerate() {
        assert_eq!(f.read(path), format!("Book {index}"));
    }
    assert_eq!(f.read("new.txt"), "Unrelated new file");
}

#[test]
fn moving_a_remote_file_to_trash_removes_the_local_copy_and_never_syncs_trash_contents() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    let mut state = f.server.state();
    let trash = f.server.create_trash(None, &mut state).unwrap();
    let node = f
        .server
        .nodes
        .iter_mut()
        .find(|node| node.name == "a.txt")
        .unwrap();
    node.parent_id = Some(trash.id);
    f.server.revision += 1;
    let report = f.run().unwrap();
    assert_eq!(report.recycled, 1);
    assert_eq!(report.downloaded, 0);
    assert!(!f.folder.path().join("a.txt").exists());
    assert!(f.history_contains(b"one"));
    let report = f.run().unwrap();
    assert_eq!(report.uploaded + report.downloaded, 0);
}

#[test]
fn a_legacy_bulwark_trash_without_a_role_is_shared_and_not_duplicated() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    let mut state = f.server.state();
    let trash = f
        .server
        .create(".Trash", None, Kind::Directory, None, 0, &mut state)
        .unwrap();
    fs::remove_file(f.folder.path().join("a.txt")).unwrap();
    f.run().unwrap();
    assert_eq!(
        f.server
            .nodes
            .iter()
            .filter(|node| node.name == ".Trash")
            .count(),
        1
    );
    let batch = f
        .server
        .nodes
        .iter()
        .find(|node| node.parent_id.as_ref() == Some(&trash.id))
        .unwrap();
    assert_eq!(f.server.node("a.txt").parent_id.as_ref(), Some(&batch.id));
}

#[test]
fn remote_delete_preserves_local_recovery_copy() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    f.server.nodes.retain(|n| n.name != "a.txt");
    f.server.revision += 1;
    f.run().unwrap();
    assert!(!f.folder.path().join("a.txt").exists());
    assert!(f.history_contains(b"one"));
}

#[test]
fn remote_delete_loses_to_unsynced_local_edit() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    f.local("a.txt", "unsaved remotely");
    f.server.nodes.retain(|n| n.name != "a.txt");
    f.server.revision += 1;
    f.run().unwrap();
    assert_eq!(f.read("a.txt"), "unsaved remotely");
    assert_eq!(
        f.server.nodes.iter().filter(|n| n.name == "a.txt").count(),
        1
    );
}

#[test]
fn remote_rename_keeps_local_edits() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    f.local("a.txt", "local edit");
    f.server
        .nodes
        .iter_mut()
        .find(|n| n.name == "a.txt")
        .unwrap()
        .name = "renamed.txt".into();
    f.server.revision += 1;
    assert_eq!(f.run().unwrap().renamed, 1);
    f.run().unwrap();
    assert_eq!(f.read("renamed.txt"), "local edit");
    assert!(!f.folder.path().join("a.txt").exists());
}

#[test]
fn local_file_rename_keeps_remote_identity() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    let id = f.server.node("a.txt").id;
    fs::rename(f.folder.path().join("a.txt"), f.folder.path().join("b.txt")).unwrap();
    assert_eq!(f.run().unwrap().renamed, 1);
    assert_eq!(f.server.node("b.txt").id, id);
    f.run().unwrap();
}

#[test]
fn disconnected_snapshot_cannot_trigger_deletions() {
    let mut f = Fixture::new();
    f.local("a", "one");
    f.run().unwrap();
    let writes = f.server.writes;
    f.server.fail_snapshot = true;
    assert!(f.run().is_err());
    assert_eq!(f.server.writes, writes);
    assert_eq!(f.read("a"), "one");
}

#[test]
fn concurrent_remote_mutation_cannot_be_overwritten() {
    let mut f = Fixture::new();
    f.local("a", "one");
    f.run().unwrap();
    let old_blob = f.server.node("a").blob_id;
    f.local("a", "local");
    f.server.race_on_upload = true;
    assert!(f.run().is_err());
    assert_eq!(f.server.node("a").blob_id, old_blob);
}

#[test]
fn missing_remote_root_is_not_an_empty_server() {
    let mut f = Fixture::new();
    f.local("a", "one");
    f.run().unwrap();
    f.server.nodes.clear();
    f.server.revision += 1;
    let writes = f.server.writes;
    assert!(f.run().is_err());
    assert_eq!(f.server.writes, writes);
    assert_eq!(f.read("a"), "one");
}

#[test]
fn profile_identity_and_single_writer_are_enforced() {
    let f = Fixture::new();
    let store = Store::open(&f.config).unwrap();
    assert!(Store::open(&f.config).is_err());
    drop(store);
    let mut changed = f.config.clone();
    changed.username = "other@example.org".into();
    assert!(Store::open(&changed).is_err());
}

#[test]
fn upgrading_root_local_state_preserves_sync_baselines_and_deletions() {
    let mut f = Fixture::new();
    f.local("document.txt", "already synchronized");
    f.local("remove.txt", "delete after migration");
    f.run().unwrap();
    let state = f.config.private_directory().unwrap();
    assert!(!state.starts_with(f.folder.path()));
    let mut cloud = f.config.clone();
    cloud.files_on_demand = true;
    assert_eq!(state, cloud.private_directory().unwrap());
    let old = f.folder.path().join(filesystem::PRIVATE_DIR);
    fs::create_dir_all(old.join("history/previous")).unwrap();
    // CI can mount application data and temporary sync roots on different
    // filesystems. Create a consistent legacy database instead of renaming
    // the external directory across those mounts.
    let db = rusqlite::Connection::open(state.join("state.sqlite")).unwrap();
    db.execute(
        "VACUUM INTO ?1",
        [old.join("state.sqlite").to_string_lossy().as_ref()],
    )
    .unwrap();
    drop(db);
    fs::remove_file(state.join("state.sqlite")).unwrap();
    fs::write(old.join("history/previous/backup.txt"), "older version").unwrap();
    let writes = f.server.writes;
    let report = f.run().unwrap();
    assert_eq!(report.uploaded + report.downloaded + report.conflicts, 0);
    assert_eq!(f.server.writes, writes);
    assert!(!old.exists());
    assert!(f.history_contains(b"older version"));
    fs::remove_file(f.folder.path().join("remove.txt")).unwrap();
    assert_eq!(f.run().unwrap().recycled, 1);
    assert_eq!(f.read("document.txt"), "already synchronized");
    assert!(!old.exists());
}

#[test]
fn file_directory_type_conflict_preserves_local_tree() {
    let mut f = Fixture::new();
    f.local("item", "one");
    f.run().unwrap();
    fs::remove_file(f.folder.path().join("item")).unwrap();
    fs::create_dir(f.folder.path().join("item")).unwrap();
    f.local("item/important", "keep me");
    assert_eq!(f.run().unwrap().conflicts, 1);
    f.run().unwrap();
    assert_eq!(f.read("item"), "one");
    assert!(walkdir::WalkDir::new(f.folder.path())
        .into_iter()
        .filter_map(Result::ok)
        .any(|e| e.file_type().is_file()
            && e.file_name() == "important"
            && fs::read(e.path()).unwrap() == b"keep me"));
}
