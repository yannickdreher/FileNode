use crate::model::{Kind, LocalEntry, LocalTree};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

pub const PRIVATE_DIR: &str = ".filenode";
pub const REMOTE_HISTORY: &str = ".filenode-history";

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 200
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        bail!("Non-portable file name: {name:?}");
    }
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        bail!("Reserved Windows file name: {name:?}");
    }
    Ok(())
}

pub fn folded(path: &str) -> String {
    path.nfc().collect::<String>().to_lowercase()
}

pub fn ignored(path: &str) -> bool {
    path.split('/').any(|part| {
        [
            PRIVATE_DIR,
            REMOTE_HISTORY,
            ".filenode-trash",
            ".trash",
            ".git",
        ]
        .contains(&part.to_lowercase().as_str())
    })
}

pub fn checked_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() || relative.len() > 240 || ignored(relative) {
        bail!("Invalid or reserved sync path: {relative:?}");
    }
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        validate_name(component)?;
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_symlink() => {
                bail!("Symbolic link in sync path: {relative}")
            }
            #[cfg(windows)]
            Ok(m) if is_reparse(&m) && !crate::cloud::is_owned(&path)? => {
                bail!("Foreign reparse point in sync path: {relative}")
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

#[cfg(windows)]
fn is_reparse(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}

pub fn hash_file(path: &Path, max_bytes: u64) -> Result<LocalEntry> {
    let mut file = open_snapshot(path)?;
    hash_snapshot(&mut file, max_bytes).with_context(|| format!("Read snapshot {}", path.display()))
}

pub(crate) fn hash_snapshot(file: &mut File, max_bytes: u64) -> Result<LocalEntry> {
    let before = file.metadata()?;
    if !before.is_file() || before.len() > max_bytes {
        bail!("File exceeds size limit or is not a regular file");
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0;
    loop {
        crate::runtime::check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > max_bytes {
            bail!("File grew beyond the upload limit");
        }
        hasher.update(&buffer[..n]);
    }
    let after = file.metadata()?;
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || total != after.len()
    {
        bail!("File changed while being read");
    }
    Ok(LocalEntry {
        kind: Kind::File,
        hash: hex::encode(hasher.finalize()),
        size: total,
    })
}

pub fn scan(root: &Path, max_bytes: u64) -> Result<LocalTree> {
    scan_with(root, |path, _| hash_file(path, max_bytes))
}

pub(crate) fn scan_with(
    root: &Path,
    read_file: impl FnMut(&Path, &str) -> Result<LocalEntry>,
) -> Result<LocalTree> {
    scan_excluding(
        root,
        &crate::exclusions::Exclusions::new(&[])?,
        &mut Default::default(),
        read_file,
    )
}

pub(crate) fn scan_excluding(
    root: &Path,
    exclusions: &crate::exclusions::Exclusions,
    omitted: &mut std::collections::HashSet<String>,
    mut read_file: impl FnMut(&Path, &str) -> Result<LocalEntry>,
) -> Result<LocalTree> {
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("Sync folder must not be a symbolic link");
    }
    let mut result = LocalTree::new();
    let mut names = HashMap::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if ignored(&e.file_name().to_string_lossy()) {
                return false;
            }
            if let Some(relative) = e.path().strip_prefix(root).ok().and_then(Path::to_str) {
                let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
                if exclusions.matches(&relative) {
                    omitted.insert(relative);
                    return false;
                }
            }
            true
        })
    {
        crate::runtime::check()?;
        let entry = entry
            .context("Read the complete local folder; deletions remain blocked on scan errors")?;
        if entry.depth() == 0 {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)?
            .to_str()
            .context("File name is not valid UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        checked_path(root, &rel)?;
        if let Some(other) = names.insert(folded(&rel), rel.clone()) {
            bail!("Windows name collision: {other} and {rel}");
        }
        let value = if entry.file_type().is_dir() {
            LocalEntry {
                kind: Kind::Directory,
                hash: "directory".into(),
                size: 0,
            }
        } else if entry.file_type().is_file() {
            read_file(entry.path(), &rel)?
        } else {
            bail!("Unsupported file type: {rel}");
        };
        result.insert(rel, value);
        if result.len() > 100_000 {
            bail!("More than 100000 local entries; refusing a partial synchronization");
        }
    }
    Ok(result)
}

/// Read-only snapshot handle. Windows denies concurrent writers while it is held;
/// Unix advisory locks only coordinate with applications honoring those locks.
pub fn open_snapshot(path: &Path) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0x1 | 0x4); // FILE_SHARE_READ | FILE_SHARE_DELETE
    }
    let file = options.open(path)?;
    fs2::FileExt::try_lock_shared(&file)?;
    Ok(file)
}

pub fn stage_snapshot(source: &Path, destination: &Path) -> Result<()> {
    use std::io::Write;
    let mut input = open_snapshot(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)?;
    let mut buffer = [0u8; 65536];
    loop {
        crate::runtime::check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()?;
    Ok(())
}

pub fn verify_local(
    root: &Path,
    path: &str,
    expected: Option<&LocalEntry>,
    max: u64,
) -> Result<()> {
    let target = checked_path(root, path)?;
    let actual = match fs::symlink_metadata(&target) {
        Ok(m) if m.is_dir() => Some(LocalEntry {
            kind: Kind::Directory,
            hash: "directory".into(),
            size: 0,
        }),
        Ok(m) if m.is_file() => Some(match crate::cloud::committed_entry(&target)? {
            Some(entry) => entry,
            None => hash_file(&target, max)?,
        }),
        Ok(_) => bail!("File type changed: {path}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if actual.as_ref() != expected {
        bail!("Local file changed; rescan required: {path}");
    }
    Ok(())
}

pub fn preserve(root: &Path, relative: &str) -> Result<PathBuf> {
    preserve_in(root, relative, &root.join(PRIVATE_DIR))
}
pub fn preserve_in(root: &Path, relative: &str, private: &Path) -> Result<PathBuf> {
    let source = checked_path(root, relative)?;
    let history = private
        .join("history")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&history)?;
    let dest = history.join(source.file_name().context("File name is missing")?);
    match fs::rename(&source, &dest) {
        Ok(()) => Ok(dest),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            // Cloud state can live on a different volume. Keep recovery moves
            // atomic by retaining that version in the root's excluded history.
            let local_private = root.join(PRIVATE_DIR);
            crate::store::ensure_state_directory(&local_private)?;
            let history = local_private
                .join("history")
                .join(uuid::Uuid::new_v4().to_string());
            fs::create_dir(&history)?;
            let dest = history.join(source.file_name().context("File name is missing")?);
            fs::rename(&source, &dest)
                .context("Preserve previous version on its original volume")?;
            Ok(dest)
        }
        Err(error) => Err(error).context("Preserve previous version for recovery"),
    }
}

pub fn install_download(root: &Path, staged: &Path, target: &Path) -> Result<()> {
    match fs::rename(staged, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            // Only a completed, verified blob crosses volumes. Copy into an
            // excluded staging file on the destination volume before publishing.
            let private = root.join(PRIVATE_DIR);
            crate::store::ensure_state_directory(&private)?;
            let mut snapshot = open_snapshot(staged)?;
            let mut adjacent = tempfile::NamedTempFile::new_in(private.join("transfers"))?;
            std::io::copy(&mut snapshot, adjacent.as_file_mut())?;
            adjacent.as_file().sync_all()?;
            adjacent
                .persist_noclobber(target)
                .context("Atomically install cross-volume download")?;
            drop(snapshot);
            fs::remove_file(staged)?;
            Ok(())
        }
        Err(error) => Err(error).context("Atomically install download"),
    }
}

pub fn conflict_path(root: &Path, path: &str) -> Result<String> {
    let p = Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let stem: String = stem.chars().take(60).collect();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let name = match p.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem} (conflict {}).{}", &suffix[..8], ext),
        None => format!("{stem} (conflict {})", &suffix[..8]),
    };
    let result = p
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| format!("{}/{name}", p.to_string_lossy()))
        .unwrap_or(name);
    if checked_path(root, &result)?.exists() {
        bail!("Conflict name already exists");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cross_platform_traversal() {
        let root = tempfile::tempdir().unwrap();
        for p in [
            "../secret",
            "/etc",
            "a/../../b",
            "C:\\data",
            "a/NUL.txt",
            "a/CON",
            "a.",
            ".filenode/db",
            "a//b",
        ] {
            assert!(checked_path(root.path(), p).is_err(), "{p}");
        }
    }
    #[test]
    fn content_hash_detects_same_size_edits() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        fs::write(&file, "one").unwrap();
        let a = hash_file(&file, 10).unwrap();
        fs::write(&file, "two").unwrap();
        assert_ne!(a.hash, hash_file(&file, 10).unwrap().hash);
    }
    #[test]
    fn case_collisions_are_rejected() {
        assert_eq!(folded("Foo/CAFÉ"), folded("foo/cafe\u{301}"));
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/tmp", root.path().join("escape")).unwrap();
        assert!(scan(root.path(), 100).is_err());
        assert!(checked_path(root.path(), "escape/file").is_err());
    }
}
