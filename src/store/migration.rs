//! Move old root-local and cloud-only state into one per-user root store.
//! Keep verified external recovery copies until the user removes them explicitly.
use super::*;
use crate::filesystem::{self, PRIVATE_DIR};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

struct Source {
    path: PathBuf,
    backup: PathBuf,
    _lock: File,
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    metadata_for_copy(path, false)
}

fn metadata_for_copy(path: &Path, _allow_cloud: bool) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                bail!("Symbolic link in internal storage: {}", path.display());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0
                    && !(_allow_cloud && crate::cloud::is_owned(path)?)
                {
                    bail!("Reparse point in internal storage: {}", path.display());
                }
            }
            Ok(Some(meta))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Inspect {}", path.display())),
    }
}

fn value(db: &Connection, key: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

fn check_identity(db: &Connection, identity: &str) -> Result<()> {
    if let Some(previous) = value(db, "identity")? {
        if serde_json::from_str::<String>(&previous)? != identity {
            bail!("This sync folder belongs to a different server, account or remote folder. Select a new local folder.");
        }
    }
    Ok(())
}

fn sync_directory(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(_path)?.sync_all()?;
    Ok(())
}

fn consistent_copy(db: &Connection, target: &Path) -> Result<()> {
    let staged = tempfile::NamedTempFile::new_in(target.parent().context("Database parent")?)?
        .into_temp_path();
    fs::remove_file(&staged)?;
    db.execute("VACUUM INTO ?1", [staged.to_string_lossy().as_ref()])?;
    let check = Connection::open(&staged)?;
    let integrity: String = check.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        bail!("Migrated database failed its integrity check: {integrity}");
    }
    drop(check);
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&staged)?
        .sync_all()?;
    fs::rename(&staged, target)?;
    sync_directory(target.parent().context("Database parent")?)?;
    Ok(())
}

fn database_file(name: &std::ffi::OsStr) -> bool {
    [
        "lock",
        "state.sqlite",
        "state.sqlite-wal",
        "state.sqlite-shm",
    ]
    .iter()
    .any(|reserved| name == *reserved)
}

fn check_database_copy(source: &Path, backup: &Path) -> Result<()> {
    let source = Connection::open(source)?;
    let backup = Connection::open(backup)?;
    for query in [
        "SELECT key,value FROM meta ORDER BY key",
        "SELECT path,value FROM baseline ORDER BY path",
        "SELECT id,action,status,timestamp FROM journal ORDER BY id",
    ] {
        let rows = |db: &Connection| -> Result<Vec<Vec<String>>> {
            let mut statement = db.prepare(query)?;
            let count = statement.column_count();
            let rows = statement.query_map([], |row| {
                (0..count)
                    .map(|column| row.get::<_, String>(column))
                    .collect()
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        if rows(&source)? != rows(&backup)? {
            bail!("Legacy database changed since its recovery copy was made; retaining both databases for recovery");
        }
    }
    Ok(())
}

pub(super) fn migrate(config: &Config, destination: &Path, identity: &str) -> Result<()> {
    migrate_from(
        destination,
        &[
            config.local_folder.join(PRIVATE_DIR),
            config.legacy_cloud_directory()?,
        ],
        identity,
    )
}

fn migrate_from(destination: &Path, paths: &[PathBuf], identity: &str) -> Result<()> {
    let database = destination.join("state.sqlite");
    if metadata(&database)?.is_some() {
        check_identity(&Connection::open(&database)?, identity)?;
    }
    let mut sources = Vec::new();
    // Hold every old engine lock until both the database and its recovery data
    // are durable. An older running engine must never write during migration.
    for path in paths {
        let Some(meta) = metadata(path)? else {
            continue;
        };
        if !meta.is_dir() {
            bail!("Internal storage is not a directory: {}", path.display());
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0x1 | 0x2 | 0x4); // Keep the lock held during cleanup.
        }
        let lock = options.open(path.join("lock"))?;
        lock.try_lock_exclusive()
            .context("Stop the previous FileNode process before migrating folder state")?;
        let key = hex::encode(Sha256::digest(serde_json::to_vec(path)?));
        let backup = destination.join("history").join("migration").join(key);
        sources.push(Source {
            path: path.clone(),
            backup,
            _lock: lock,
        });
    }
    if sources.is_empty() {
        return Ok(());
    }
    // Cloud state supersedes the old full-sync database unless an older release
    // detached it and copied the newer full-sync baseline back into the root.
    let mut selected = None;
    for (index, source) in sources.iter().enumerate() {
        let db_path = source.path.join("state.sqlite");
        if metadata(&db_path)?.is_some() {
            let db = Connection::open(&db_path)?;
            check_identity(&db, identity)?;
            let detached = value(&db, "cloud_detached")?.as_deref() == Some("true");
            if selected.is_none() || !detached {
                selected = Some(index);
            }
        }
    }
    for source in &sources {
        fs::create_dir_all(source.backup.join("contents"))?;
        let complete = source.backup.join("complete");
        if !complete.try_exists()? {
            if metadata(&source.path.join("state.sqlite"))?.is_some() {
                let db = Connection::open(source.path.join("state.sqlite"))?;
                consistent_copy(&db, &source.backup.join("database.sqlite"))?;
            }
            for entry in fs::read_dir(&source.path)? {
                let entry = entry?;
                if !database_file(&entry.file_name()) {
                    copy_tree(
                        &entry.path(),
                        &source.backup.join("contents").join(entry.file_name()),
                    )?;
                }
            }
            let marker = File::create(&complete)?;
            marker.sync_all()?;
            sync_directory(&source.backup)?;
        }
        if metadata(&source.path.join("state.sqlite"))?.is_some() {
            check_database_copy(
                &source.path.join("state.sqlite"),
                &source.backup.join("database.sqlite"),
            )?;
        }
    }
    if !database.try_exists()? {
        if let Some(index) = selected {
            let source = &sources[index];
            let transfers = source.backup.join("contents/transfers");
            if transfers.try_exists()? {
                copy_tree(&transfers, &destination.join("transfers"))?;
            }
            let db = Connection::open(source.backup.join("database.sqlite"))?;
            // Publish the database last. A failed copy can be retried without
            // creating an empty baseline or losing in-flight download offsets.
            consistent_copy(&db, &database)?;
        }
    }
    // Once the external database exists it remains authoritative on retry.
    // Cleanup only removes files proven present in the durable recovery copy;
    // unknown/new files and links stop cleanup rather than being discarded.
    for source in &sources {
        for entry in fs::read_dir(&source.path)? {
            let entry = entry?;
            if !database_file(&entry.file_name()) {
                remove_backed_up_tree(
                    &entry.path(),
                    &source.backup.join("contents").join(entry.file_name()),
                )?;
            }
        }
        if metadata(&source.path.join("state.sqlite"))?.is_some() {
            let db = Connection::open(source.path.join("state.sqlite"))?;
            db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        for name in [
            "state.sqlite-wal",
            "state.sqlite-shm",
            "state.sqlite",
            "lock",
        ] {
            let path = source.path.join(name);
            if metadata(&path)?.is_some() {
                fs::remove_file(path)?;
            }
        }
        fs::remove_dir(&source.path).context("Remove migrated internal directory")?;
    }
    Ok(())
}

/// This also handles migrations across volumes. Never follow links or truncate
/// an existing recovery copy; retrying a partially copied tree is idempotent.
pub(crate) fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    copy_tree_with_cloud(source, target, false)
}

pub(crate) fn copy_recovery_tree(source: &Path, target: &Path) -> Result<()> {
    copy_tree_with_cloud(source, target, true)
}

fn copy_tree_with_cloud(source: &Path, target: &Path, allow_cloud: bool) -> Result<()> {
    let meta = metadata_for_copy(source, allow_cloud)?.context("Migration source disappeared")?;
    let existing = metadata(target)?;
    if meta.is_dir() {
        if existing.as_ref().is_some_and(|meta| !meta.is_dir()) {
            bail!("Recovery path is not a directory");
        }
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree_with_cloud(&entry.path(), &target.join(entry.file_name()), allow_cloud)?;
        }
        sync_directory(target)?;
        sync_directory(target.parent().context("Recovery parent")?)?;
    } else if meta.is_file() {
        let mut input = filesystem::open_snapshot(source)?;
        let expected = filesystem::hash_snapshot(&mut input, u64::MAX)?;
        if existing.is_some() {
            if filesystem::hash_file(target, u64::MAX)? != expected {
                bail!(
                    "Recovery copy differs; retaining the original: {}",
                    source.display()
                );
            }
        } else {
            use std::io::{Seek, SeekFrom};
            input.seek(SeekFrom::Start(0))?;
            let mut staged =
                tempfile::NamedTempFile::new_in(target.parent().context("Recovery parent")?)?;
            std::io::copy(&mut input, staged.as_file_mut())?;
            staged.as_file().sync_all()?;
            staged.as_file_mut().seek(SeekFrom::Start(0))?;
            if filesystem::hash_snapshot(staged.as_file_mut(), u64::MAX)? != expected {
                bail!("Recovery copy verification failed");
            }
            input.seek(SeekFrom::Start(0))?;
            if filesystem::hash_snapshot(&mut input, u64::MAX)? != expected {
                bail!("Source changed while copying; retaining the original");
            }
            staged
                .persist_noclobber(target)
                .context("Publish recovery copy")?;
            sync_directory(target.parent().context("Recovery parent")?)?;
        }
    } else {
        bail!(
            "Unsupported entry in internal storage: {}",
            source.display()
        );
    }
    Ok(())
}

pub(crate) fn remove_backed_up_tree(source: &Path, backup: &Path) -> Result<()> {
    remove_tree_with_cloud(source, backup, false)
}

pub(crate) fn remove_recovered_tree(source: &Path, backup: &Path) -> Result<()> {
    remove_tree_with_cloud(source, backup, true)
}

fn remove_tree_with_cloud(source: &Path, backup: &Path, allow_cloud: bool) -> Result<()> {
    let meta = metadata_for_copy(source, allow_cloud)?.context("Recovery source disappeared")?;
    metadata(backup)?.context("Recovery backup is missing")?;
    if meta.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            remove_tree_with_cloud(&entry.path(), &backup.join(entry.file_name()), allow_cloud)?;
        }
        // Never recursively remove a tree: a newly created entry must survive.
        fs::remove_dir(source)?;
    } else if meta.is_file() {
        let mut input = filesystem::open_snapshot(source)?;
        if filesystem::hash_snapshot(&mut input, u64::MAX)?
            != filesystem::hash_file(backup, u64::MAX)?
        {
            bail!(
                "Source changed since backup; retaining it: {}",
                source.display()
            );
        }
        fs::remove_file(source)?;
    } else {
        bail!("Unsafe recovery source");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(path: &Path, generation: &str, detached: bool) -> Connection {
        fs::create_dir_all(path.join("history/previous")).unwrap();
        fs::create_dir_all(path.join("transfers")).unwrap();
        fs::write(path.join("history/previous/document.txt"), generation).unwrap();
        fs::write(path.join("transfers/blob.part"), "partial download").unwrap();
        fs::write(path.join("unrecognized.txt"), "also keep this").unwrap();
        let db = Connection::open(path.join("state.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE baseline (path TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE journal (id TEXT PRIMARY KEY, action TEXT NOT NULL, status TEXT NOT NULL, timestamp TEXT NOT NULL);").unwrap();
        db.execute(
            "INSERT INTO meta VALUES ('identity', ?1)",
            [serde_json::to_string("identity").unwrap()],
        )
        .unwrap();
        db.execute("INSERT INTO meta VALUES ('generation', ?1)", [generation])
            .unwrap();
        db.execute(
            "INSERT INTO meta VALUES ('cloud_detached', ?1)",
            [detached.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO baseline VALUES ('document.txt', ?1)",
            [generation],
        )
        .unwrap();
        db.execute(
            "INSERT INTO journal VALUES ('pending', 'upload', 'pending', 'today')",
            [],
        )
        .unwrap();
        db
    }

    fn generation(path: &Path) -> String {
        value(
            &Connection::open(path.join("state.sqlite")).unwrap(),
            "generation",
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn migrates_full_state_and_history_and_removes_the_old_directory() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("sync/.filenode");
        let new = temp.path().join("appdata/root");
        fs::create_dir_all(&new).unwrap();
        drop(seed(&old, "full", false));
        migrate_from(&new, std::slice::from_ref(&old), "identity").unwrap();
        assert!(!old.exists());
        assert_eq!(generation(&new), "full");
        assert_eq!(
            fs::read(new.join("transfers/blob.part")).unwrap(),
            b"partial download"
        );
        let db = Connection::open(new.join("state.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM journal WHERE id='pending'", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
            "pending"
        );
        let backup_files: Vec<_> = walkdir::WalkDir::new(new.join("history"))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .collect();
        assert!(backup_files
            .iter()
            .any(|e| e.file_name() == "unrecognized.txt"
                && fs::read(e.path()).unwrap() == b"also keep this"));
        assert!(backup_files
            .iter()
            .any(|e| e.file_name() == "document.txt" && fs::read(e.path()).unwrap() == b"full"));
        migrate_from(&new, &[old], "identity").unwrap();
        assert_eq!(generation(&new), "full");
    }

    #[test]
    fn selects_active_cloud_state_or_the_newer_detached_full_state() {
        for detached in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let old = temp.path().join("sync/.filenode");
            let cloud = temp.path().join("appdata/cloud");
            let new = temp.path().join("appdata/new");
            fs::create_dir_all(&new).unwrap();
            drop(seed(&old, "full", false));
            drop(seed(&cloud, "cloud", detached));
            migrate_from(&new, &[old.clone(), cloud.clone()], "identity").unwrap();
            assert_eq!(generation(&new), if detached { "full" } else { "cloud" });
            assert!(!old.exists());
            assert!(!cloud.exists());
            let recovered: Vec<_> = walkdir::WalkDir::new(new.join("history"))
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_name() == "document.txt")
                .map(|e| fs::read(e.path()).unwrap())
                .collect();
            assert!(
                recovered.contains(&b"full".to_vec()) && recovered.contains(&b"cloud".to_vec())
            );
        }
    }

    #[test]
    fn refuses_running_legacy_engine_and_wrong_identity_without_removing_state() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("sync/.filenode");
        let new = temp.path().join("appdata");
        fs::create_dir_all(&new).unwrap();
        drop(seed(&old, "original", false));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(old.join("lock"))
            .unwrap();
        lock.try_lock_exclusive().unwrap();
        assert!(migrate_from(&new, std::slice::from_ref(&old), "identity")
            .unwrap_err()
            .to_string()
            .contains("previous FileNode"));
        drop(lock);
        assert!(migrate_from(&new, std::slice::from_ref(&old), "other account").is_err());
        assert_eq!(generation(&old), "original");
        assert!(!new.join("state.sqlite").exists());
    }

    #[test]
    fn cleanup_retry_never_replaces_the_new_external_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("sync/.filenode");
        let new = temp.path().join("appdata");
        fs::create_dir_all(&new).unwrap();
        drop(seed(&old, "old", false));
        drop(seed(&new, "newer", false));
        migrate_from(&new, &[old], "identity").unwrap();
        assert_eq!(generation(&new), "newer");
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_cleanup_resumes_without_reimporting_stale_baselines() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("sync/.filenode");
        let new = temp.path().join("appdata");
        fs::create_dir(&new).unwrap();
        drop(seed(&old, "old", false));
        // Simulate a file preventing cleanup after the new database is durable.
        let blocked = OpenOptions::new()
            .read(true)
            .share_mode(0x1)
            .open(old.join("history/previous/document.txt"))
            .unwrap();
        assert!(migrate_from(&new, std::slice::from_ref(&old), "identity").is_err());
        assert_eq!(generation(&new), "old");
        assert!(old.exists());
        let db = Connection::open(new.join("state.sqlite")).unwrap();
        db.execute("UPDATE meta SET value='newer' WHERE key='generation'", [])
            .unwrap();
        drop(db);
        drop(blocked);
        migrate_from(&new, std::slice::from_ref(&old), "identity").unwrap();
        assert!(!old.exists());
        assert_eq!(generation(&new), "newer");
    }

    #[test]
    fn consistent_database_copy_includes_uncheckpointed_wal_transactions() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let db = seed(&old, "wal generation", false);
        assert!(old.join("state.sqlite-wal").metadata().unwrap().len() > 0);
        let new = temp.path().join("new");
        fs::create_dir(&new).unwrap();
        consistent_copy(&db, &new.join("state.sqlite")).unwrap();
        check_database_copy(&old.join("state.sqlite"), &new.join("state.sqlite")).unwrap();
        assert_eq!(generation(&new), "wal generation");
    }

    #[test]
    fn copy_retry_preserves_sources_and_detects_changed_or_missing_backups() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let copy = temp.path().join("backup");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("a"), "original").unwrap();
        copy_tree(&old, &copy).unwrap();
        copy_tree(&old, &copy).unwrap();
        fs::write(old.join("a"), "changed").unwrap();
        assert!(remove_backed_up_tree(&old, &copy).is_err());
        assert_eq!(fs::read(old.join("a")).unwrap(), b"changed");
        assert_eq!(fs::read(copy.join("a")).unwrap(), b"original");
        assert!(copy_tree(&old, &copy).is_err());
        fs::remove_file(copy.join("a")).unwrap();
        assert!(remove_backed_up_tree(&old, &copy).is_err());
        assert!(old.join("a").exists());
    }

    #[cfg(unix)]
    #[test]
    fn migration_never_follows_links_or_deletes_their_targets() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        fs::create_dir(&new).unwrap();
        drop(seed(&old, "safe", false));
        let outside = temp.path().join("outside");
        fs::write(&outside, "keep me").unwrap();
        std::os::unix::fs::symlink(&outside, old.join("escape")).unwrap();
        assert!(migrate_from(&new, std::slice::from_ref(&old), "identity").is_err());
        assert_eq!(generation(&old), "safe");
        assert_eq!(fs::read(&outside).unwrap(), b"keep me");
        assert!(!new.join("state.sqlite").exists());
    }
}
