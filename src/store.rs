use crate::{config::Config, filesystem::PRIVATE_DIR, model::*};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub struct Store {
    pub db: Connection,
    _lock: File,
}

impl Store {
    pub fn open(config: &Config) -> Result<Self> {
        #[cfg(windows)]
        if !config.files_on_demand && crate::cloud::is_registered(config)? {
            bail!("This folder is still registered for Files On-Demand. Disable it in FileNode first, after making every file available locally.");
        }
        let folder = config.private_directory()?;
        if let Ok(meta) = fs::symlink_metadata(&folder) {
            if !meta.is_dir() || meta.file_type().is_symlink() {
                bail!("Unsafe state directory");
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    bail!("State directory is a reparse point");
                }
            }
        }
        fs::create_dir_all(&folder)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(folder.join("lock"))?;
        lock.try_lock_exclusive()
            .context("Another FileNode process is using this folder")?;
        let detached = if config.files_on_demand && folder.join("state.sqlite").exists() {
            let previous = Connection::open(folder.join("state.sqlite"))?;
            previous
                .query_row(
                    "SELECT value FROM meta WHERE key='cloud_detached'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .as_deref()
                == Some("true")
        } else {
            false
        };
        if config.files_on_demand && (!folder.join("state.sqlite").exists() || detached) {
            // Copy a consistent database under the legacy root's exclusive
            // lock. Keep the original database/history intact for recovery.
            let legacy = config.local_folder.join(PRIVATE_DIR);
            if legacy.join("state.sqlite").exists() {
                ensure_state_directory(&legacy)?;
                let legacy_lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(legacy.join("lock"))?;
                legacy_lock
                    .try_lock_exclusive()
                    .context("Stop the previous sync engine before enabling Files On-Demand")?;
                let source = Connection::open(legacy.join("state.sqlite"))?;
                let staged = tempfile::NamedTempFile::new_in(&folder)?.into_temp_path();
                fs::remove_file(&staged)?;
                source.execute("VACUUM INTO ?1", [staged.to_string_lossy().as_ref()])?;
                if folder.join("state.sqlite").exists() {
                    let previous = Connection::open(folder.join("state.sqlite"))?;
                    previous.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
                }
                fs::rename(&staged, folder.join("state.sqlite"))?;
            }
        }
        let db = Connection::open(folder.join("state.sqlite"))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS baseline (path TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS journal (id TEXT PRIMARY KEY, action TEXT NOT NULL, status TEXT NOT NULL, timestamp TEXT NOT NULL);")?;
        let store = Self { db, _lock: lock };
        if config.files_on_demand {
            store.set_meta("cloud_detached", &false)?;
        }
        let identity = format!(
            "{}|{}|{}|{}|{}",
            config.server.trim_end_matches('/'),
            config.username,
            config.account_id.as_deref().unwrap_or(""),
            config.local_folder.canonicalize()?.display(),
            config.remote_folder_path()?
        );
        if let Some(previous) = store.get_meta::<String>("identity")? {
            if previous != identity {
                bail!("This sync folder belongs to a different server, account or remote folder. Select a new local folder.");
            }
        } else {
            store.set_meta("identity", &identity)?;
        }
        Ok(store)
    }

    pub fn baselines(&self) -> Result<Baselines> {
        let mut stmt = self.db.prepare("SELECT path,value FROM baseline")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut result = Baselines::new();
        for row in rows {
            let (p, v) = row?;
            result.insert(p, serde_json::from_str(&v)?);
        }
        Ok(result)
    }
    pub fn record(&self, path: &str, local: &LocalEntry, remote: &Node) -> Result<()> {
        let b = Baseline {
            remote_id: remote.id.clone(),
            remote_blob: remote.blob_id.clone(),
            local: local.clone(),
        };
        self.db.execute("INSERT INTO baseline(path,value) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET value=excluded.value", params![path, serde_json::to_string(&b)?])?;
        Ok(())
    }
    pub fn forget(&self, path: &str) -> Result<()> {
        self.db
            .execute("DELETE FROM baseline WHERE path=?1", [path])?;
        Ok(())
    }
    pub fn forget_tree(&self, path: &str) -> Result<()> {
        for p in self.baselines()?.keys() {
            if p == path || p.starts_with(&format!("{path}/")) {
                self.forget(p)?;
            }
        }
        Ok(())
    }
    pub fn get_meta<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .db
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn set_meta<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.db.execute("INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn begin(&self, action: &Action) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.db.execute(
            "INSERT INTO journal VALUES(?1,?2,'pending',?3)",
            params![
                id,
                serde_json::to_string(action)?,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(id)
    }
    pub fn done(&self, id: &str) -> Result<()> {
        self.db
            .execute("UPDATE journal SET status='done' WHERE id=?1", [id])?;
        // Retain the last 1000 completed operations; interrupted operations are never pruned.
        self.db.execute("DELETE FROM journal WHERE status='done' AND rowid NOT IN (SELECT rowid FROM journal ORDER BY rowid DESC LIMIT 1000)", [])?;
        Ok(())
    }
    pub fn rename_baseline(&mut self, old: &str, new: &str) -> Result<()> {
        let rows = self.baselines()?;
        let tx = self.db.transaction()?;
        for (path, baseline) in rows {
            if path == old || path.starts_with(&format!("{old}/")) {
                let dest = format!("{new}{}", &path[old.len()..]);
                tx.execute("DELETE FROM baseline WHERE path=?1", [&path])?;
                tx.execute(
                    "INSERT INTO baseline(path,value) VALUES(?1,?2)",
                    params![dest, serde_json::to_string(&baseline)?],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

pub fn ensure_private_path(root: &Path) -> Result<()> {
    ensure_state_directory(&root.join(PRIVATE_DIR))
}

pub fn ensure_state_directory(folder: &Path) -> Result<()> {
    for path in [
        folder.to_path_buf(),
        folder.join("history"),
        folder.join("transfers"),
    ] {
        if let Ok(m) = fs::symlink_metadata(&path) {
            if m.file_type().is_symlink() || !m.is_dir() {
                bail!("Unsafe internal directory");
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    bail!("Unsafe internal reparse directory");
                }
            }
        }
        fs::create_dir_all(path)?;
    }
    Ok(())
}
