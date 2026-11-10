use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(super) const CHUNK: u64 = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub ino: u64,
    pub path: Option<String>,
    pub kind: Kind,
    pub size: u64,
    pub modified: u64,
    pub mode: u16,
    pub identity: Option<Identity>,
    pub dirty: bool,
    pub pinned: bool,
    pub chunks: BTreeSet<u64>,
}
impl Entry {
    pub fn parent(&self) -> Option<&str> {
        self.path
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| p.rsplit_once('/').map(|p| p.0).unwrap_or(""))
    }
    pub fn new(ino: u64, path: String, kind: Kind) -> Self {
        Self {
            ino,
            path: Some(path),
            kind,
            size: 0,
            modified: now(),
            mode: if kind == Kind::Directory {
                0o700
            } else {
                0o600
            },
            identity: None,
            dirty: true,
            pinned: false,
            chunks: BTreeSet::new(),
        }
    }
    pub fn complete(&self) -> bool {
        self.kind == Kind::Directory
            || self.dirty
            || (0..self.size.div_ceil(CHUNK)).all(|n| self.chunks.contains(&n))
    }
    pub fn status(&self) -> &'static str {
        if self.dirty {
            "syncing"
        } else if self.pinned && self.complete() {
            "pinned"
        } else if self.complete() {
            "local"
        } else {
            "online"
        }
    }
}

pub(super) struct Catalog {
    pub db: Connection,
}
impl Catalog {
    pub fn open(path: &Path, key: &str) -> Result<Self> {
        let db = Connection::open(path)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS entries (ino INTEGER PRIMARY KEY, path TEXT UNIQUE, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS identity (key TEXT NOT NULL);")?;
        let old: Option<String> = db
            .query_row("SELECT key FROM identity", [], |r| r.get(0))
            .optional()?;
        if let Some(old) = old {
            if old != key {
                bail!("Cloud cache belongs to another account or remote folder");
            }
        } else {
            db.execute("INSERT INTO identity VALUES (?1)", [key])?;
        }
        let this = Self { db };
        let has_parent = {
            let mut statement = this.db.prepare("PRAGMA table_info(entries)")?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            names.iter().any(|name| name == "parent")
        };
        if !has_parent {
            let tx = this.db.unchecked_transaction()?;
            tx.execute("ALTER TABLE entries ADD COLUMN parent TEXT", [])?;
            for entry in this.entries()? {
                tx.execute(
                    "UPDATE entries SET parent=?2 WHERE ino=?1",
                    params![entry.ino, entry.parent()],
                )?;
            }
            tx.commit()?;
        }
        this.db.execute(
            "CREATE INDEX IF NOT EXISTS entries_parent ON entries(parent)",
            [],
        )?;
        if this.entry(1)?.is_none() {
            let mut root = Entry::new(1, String::new(), Kind::Directory);
            root.dirty = false;
            this.save(&root)?;
        }
        Ok(this)
    }
    pub fn entry(&self, ino: u64) -> Result<Option<Entry>> {
        let row: Option<String> = self
            .db
            .query_row("SELECT value FROM entries WHERE ino=?1", [ino], |r| {
                r.get(0)
            })
            .optional()?;
        row.map(|row| serde_json::from_str(&row).map_err(Into::into))
            .transpose()
    }
    pub fn path(&self, path: &str) -> Result<Option<Entry>> {
        let row: Option<String> = self
            .db
            .query_row("SELECT value FROM entries WHERE path=?1", [path], |r| {
                r.get(0)
            })
            .optional()?;
        row.map(|row| serde_json::from_str(&row).map_err(Into::into))
            .transpose()
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut query = self
            .db
            .prepare("SELECT value FROM entries WHERE path IS NOT NULL ORDER BY path")?;
        let rows = query.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn save(&self, entry: &Entry) -> Result<()> {
        self.db.execute("INSERT INTO entries (ino,path,value,parent) VALUES (?1,?2,?3,?4) ON CONFLICT(ino) DO UPDATE SET path=excluded.path,value=excluded.value,parent=excluded.parent",
            params![entry.ino, entry.path, serde_json::to_string(entry)?, entry.parent()])?;
        Ok(())
    }
    pub fn children(&self, path: &str) -> Result<Vec<Entry>> {
        let mut statement = self
            .db
            .prepare("SELECT value FROM entries WHERE parent=?1 ORDER BY path")?;
        let rows = statement.query_map([path], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn allocate(&self, path: &str, kind: Kind) -> Result<Entry> {
        let ino = self
            .db
            .query_row("SELECT COALESCE(MAX(ino),1)+1 FROM entries", [], |r| {
                r.get(0)
            })?;
        let entry = Entry::new(ino, path.to_owned(), kind);
        Ok(entry)
    }
}
