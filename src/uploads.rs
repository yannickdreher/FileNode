//! Bounded parallel blob transfer, followed by serial conditional FileNode
//! commits in the engine. Uncommitted blobs expire through the server's TTL.
use crate::{config::Config, filesystem as disk, jmap::Uploader, model::LocalEntry};
use anyhow::{bail, Result};
use std::{collections::HashMap, sync::Arc};

pub fn stage(config: &Config, path: &str, expected: &LocalEntry) -> Result<tempfile::TempPath> {
    disk::verify_local(
        &config.local_folder,
        path,
        Some(expected),
        config.file_limit(),
    )?;
    let source = disk::checked_path(&config.local_folder, path)?;
    let staged = tempfile::NamedTempFile::new_in(
        config
            .local_folder
            .join(disk::PRIVATE_DIR)
            .join("transfers"),
    )?
    .into_temp_path();
    disk::stage_snapshot(&source, &staged)?;
    if &disk::hash_file(&staged, config.file_limit())? != expected {
        bail!("File changed while preparing upload: {path}");
    }
    Ok(staged)
}

pub struct Prepared {
    pub blob: Option<Result<String>>,
    pub committed: bool,
    path: String,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.committed {
            crate::activity::notice(&self.path, "upload", "retrying");
        }
    }
}

pub fn prepare(
    config: &Config,
    uploader: Arc<dyn Uploader>,
    files: Vec<(String, LocalEntry)>,
) -> HashMap<String, Prepared> {
    let context = crate::activity::Scope::capture();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = files
            .into_iter()
            .map(|(path, expected)| {
                let context = context.clone();
                let uploader = uploader.clone();
                let key = path.clone();
                let worker = scope.spawn(move || {
                    let _runtime = crate::runtime::Scope::enter(uploader.policy());
                    let _activity_scope = context.enter();
                    let mut activity =
                        crate::activity::FileActivity::begin(&path, "upload", expected.size);
                    let result = (|| {
                        let staged = stage(config, &path, &expected)?;
                        uploader.send_blob(&staged, expected.size)
                    })();
                    if result.is_ok() {
                        activity.finish("queued");
                    }
                    result
                });
                (key, worker)
            })
            .collect();
        jobs.into_iter()
            .map(|(path, worker)| {
                let blob = worker.join().unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("Upload worker terminated unexpectedly"))
                });
                (
                    path.clone(),
                    Prepared {
                        blob: Some(blob),
                        committed: false,
                        path,
                    },
                )
            })
            .collect()
    })
}
