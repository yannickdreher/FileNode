use super::*;
use std::sync::atomic::AtomicUsize;

struct Bytes {
    data: Vec<u8>,
    requests: AtomicUsize,
    fail: AtomicBool,
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
        if cancel.load(Ordering::SeqCst) || self.fail.load(Ordering::SeqCst) {
            bail!("Simulated disconnected laptop");
        }
        consume(&self.data[offset as usize..(offset + length) as usize])
    }
}
struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        shutdown();
    }
}

#[test]
#[ignore = "Verifies metadata stays responsive during four blocked native FUSE downloads"]
fn native_slow_downloads_do_not_block_file_manager_metadata() -> Result<()> {
    struct Slow {
        entered: AtomicUsize,
        release: AtomicBool,
    }
    impl Downloader for Slow {
        fn read_range(
            &self,
            _: &Node,
            _: u64,
            _: u64,
            cancel: &AtomicBool,
            consume: &mut dyn FnMut(&[u8]) -> Result<()>,
        ) -> Result<()> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !self.release.load(Ordering::SeqCst) {
                if cancel.load(Ordering::SeqCst) || std::time::Instant::now() > deadline {
                    bail!("Cancelled blocked test download");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            consume(b"x")
        }
    }
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("Cloud");
    fs::create_dir(&root)?;
    let config = Config {
        local_folder: root.clone(),
        username: "test@example.org".into(),
        files_on_demand: true,
        ..Config::default()
    };
    let _cleanup = Cleanup;
    let bytes = Arc::new(Slow {
        entered: AtomicUsize::new(0),
        release: AtomicBool::new(false),
    });
    let provider = Provider::connect(&config, bytes.clone())?;
    let mut readers = Vec::new();
    for number in 0..4 {
        let name = format!("file{number}.txt");
        let node = Node {
            id: name.clone(),
            name: name.clone(),
            parent_id: None,
            blob_id: Some(name.clone()),
            kind: Kind::File,
            size: 1,
            role: None,
        };
        provider.publish(&name, &node, None)?;
        let path = root.join(name);
        readers.push(std::thread::spawn(move || fs::read(path)));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while bytes.entered.load(Ordering::SeqCst) < 4 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(bytes.entered.load(Ordering::SeqCst), 4);
    let start = std::time::Instant::now();
    assert_eq!(fs::read_dir(&root)?.count(), 4);
    assert_eq!(fs::metadata(root.join("file0.txt"))?.len(), 1);
    assert_eq!(
        command("status", &root.join("file0.txt"))?["state"],
        "online"
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    bytes.release.store(true, Ordering::SeqCst);
    for reader in readers {
        assert_eq!(reader.join().unwrap()?, b"x");
    }
    Ok(())
}

#[test]
#[ignore = "Mounts a real per-user FUSE filesystem; needs /dev/fuse and fusermount3"]
fn native_mount_ranges_pin_eviction_restart_and_materialization() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("Documents");
    fs::create_dir(&root)?;
    let config = Config {
        local_folder: root.clone(),
        username: "test@example.org".into(),
        files_on_demand: true,
        ..Config::default()
    };
    let _cleanup = Cleanup;
    let bytes = Arc::new(Bytes {
        data: (0..CHUNK + 37).map(|i| (i % 251) as u8).collect(),
        requests: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    let node = Node {
        id: "id".into(),
        name: "document.bin".into(),
        parent_id: None,
        blob_id: Some("blob".into()),
        kind: Kind::File,
        size: bytes.data.len() as u64,
        role: None,
    };
    let provider = Provider::connect(&config, bytes.clone())?;
    let (events, received) = std::sync::mpsc::channel();
    use notify::Watcher;
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = events.send(event);
    })?;
    watcher.watch(&root, notify::RecursiveMode::Recursive)?;
    let local = provider.publish("document.bin", &node, None)?;
    let path = root.join("document.bin");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let event = received
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))??;
        if matches!(event.kind, notify::EventKind::Create(_)) && event.paths.contains(&path) {
            break;
        }
    }
    let store = crate::store::Store::open(&config)?;
    store.record("document.bin", &local, &node)?;
    drop(store);
    assert_eq!(fs::metadata(&path)?.len(), node.size);
    assert_eq!(fs::read_dir(&root)?.count(), 1);
    assert_eq!(committed_entry(&path)?, Some(local.clone()));
    assert_eq!(
        bytes.requests.load(Ordering::SeqCst),
        0,
        "metadata hydrated contents"
    );
    assert_eq!(command("status", &path)?["state"], "online");
    assert!(command("evict", &temp.path().join("elsewhere")).is_err());
    {
        let mut file = File::open(&path)?;
        file.seek(SeekFrom::Start(CHUNK + 3))?;
        let mut data = [0u8; 12];
        file.read_exact(&mut data)?;
        assert_eq!(
            data.as_slice(),
            &bytes.data[CHUNK as usize + 3..CHUNK as usize + 15]
        );
    }
    assert_eq!(
        bytes.requests.load(Ordering::SeqCst),
        1,
        "nonzero read fetched the preceding chunk"
    );
    bytes.fail.store(true, Ordering::SeqCst);
    assert!(provider.state.availability("document.bin", true).is_err());
    bytes.fail.store(false, Ordering::SeqCst);
    provider.state.availability("document.bin", true)?;
    assert_eq!(
        bytes.requests.load(Ordering::SeqCst),
        3,
        "retry redownloaded a persisted range"
    );
    assert_eq!(fs::read(&path)?, bytes.data);
    assert_eq!(command("status", &path)?["state"], "pinned");
    drop(provider);
    shutdown();
    assert_eq!(fs::read_dir(&root)?.count(), 0);
    let provider = Provider::connect(&config, bytes.clone())?;
    assert_eq!(fs::read(&path)?, bytes.data);
    assert_eq!(
        bytes.requests.load(Ordering::SeqCst),
        3,
        "restart lost cached ranges"
    );
    provider.state.availability("document.bin", false)?;
    assert_eq!(command("status", &path)?["state"], "online");
    fs::write(&path, b"edited locally")?;
    assert!(committed_entry(&path)?.is_none());
    assert!(provider.state.availability("document.bin", false).is_err());
    let uploaded = crate::filesystem::hash_file(&path, u64::MAX)?;
    let updated = Node {
        blob_id: Some("uploaded-blob".into()),
        size: uploaded.size,
        ..node
    };
    provider.publish("document.bin", &updated, Some(&uploaded))?;
    assert_eq!(
        fs::read(&path)?,
        b"edited locally",
        "commit discarded uploaded contents"
    );
    fs::create_dir(root.join("nested"))?;
    provider.state.availability("nested", true)?;
    fs::write(root.join("nested/pinned-local.txt"), b"new pinned child")?;
    assert!(
        provider
            .state
            .catalog
            .lock()
            .unwrap()
            .path("nested/pinned-local.txt")?
            .unwrap()
            .pinned
    );
    fs::rename(&path, root.join("nested/renamed.bin"))?;
    let mut open = File::open(root.join("nested/renamed.bin"))?;
    fs::remove_file(root.join("nested/renamed.bin"))?;
    let mut retained = Vec::new();
    open.read_to_end(&mut retained)?;
    assert_eq!(retained, b"edited locally");
    drop(open);
    fs::write(root.join("new.txt"), b"offline local file")?;
    assert!(fs::read_dir(&root)?.count() > 0);
    drop(provider);
    detach(&config)?;
    assert!(!is_registered(&config)?);
    assert_eq!(fs::read(root.join("new.txt"))?, b"offline local file");
    Ok(())
}

#[test]
fn catalog_retains_metadata_without_claiming_cached_bytes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let db = Catalog::open(&temp.path().join("namespace.sqlite"), "test-root")?;
    let mut entry = db.allocate("document.txt", Kind::File)?;
    entry.size = CHUNK + 7;
    entry.dirty = false;
    assert_eq!(entry.status(), "online");
    entry.chunks.insert(0);
    assert!(!entry.complete());
    entry.chunks.insert(1);
    assert!(entry.complete());
    assert_eq!(entry.status(), "local");
    entry.pinned = true;
    assert_eq!(entry.status(), "pinned");
    db.save(&entry)?;
    drop(db);
    let db = Catalog::open(&temp.path().join("namespace.sqlite"), "test-root")?;
    assert_eq!(db.path("document.txt")?.unwrap().status(), "pinned");
    assert!(Catalog::open(&temp.path().join("namespace.sqlite"), "different-account").is_err());
    Ok(())
}
