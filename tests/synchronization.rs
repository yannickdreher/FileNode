//! Stateful integration tests: real filesystem + SQLite, simulated immutable
//! JMAP blobs and global ifInState. No production account is ever contacted.
use anyhow::{bail, Result};
use filenode::{config::Config, engine, filesystem, jmap::Remote, model::*, store::Store};
use std::{collections::HashMap, fs, path::Path, sync::atomic::AtomicBool};

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
        walkdir::WalkDir::new(
            self.folder
                .path()
                .join(filesystem::PRIVATE_DIR)
                .join("history"),
        )
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
fn local_delete_soft_moves_remote_to_recovery_folder() {
    let mut f = Fixture::new();
    f.local("a.txt", "one");
    f.run().unwrap();
    let id = f.server.node("a.txt").id;
    fs::remove_file(f.folder.path().join("a.txt")).unwrap();
    assert_eq!(f.run().unwrap().recycled, 1);
    let trash = f.server.node(filesystem::REMOTE_TRASH).id;
    assert_eq!(
        f.server
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .parent_id
            .as_ref(),
        Some(&trash)
    );
    f.run().unwrap();
    assert!(!f.folder.path().join("a.txt").exists());
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
