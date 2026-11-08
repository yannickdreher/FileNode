use crate::{
    config::Config,
    filesystem as disk,
    i18n::Message,
    jmap::{self, Remote},
    model::*,
    planner,
    store::{self, Store},
};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct Report {
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: usize,
    pub recycled: usize,
    pub blocked: usize,
    pub renamed: usize,
    pub needs_rescan: bool,
    pub deferred: usize,
    pub failed_folders: usize,
}

pub fn sync<R: Remote>(
    config: &Config,
    store: &mut Store,
    remote: &mut R,
    cancel: &AtomicBool,
    log: &mut dyn FnMut(Message),
) -> Result<Report> {
    let mut scanner =
        crate::scanner::LocalScanner::new(&config.local_folder, config.full_scan_seconds);
    sync_with_scanner(config, store, remote, cancel, log, &mut scanner)
}

pub fn sync_with_scanner<R: Remote>(
    config: &Config,
    store: &mut Store,
    remote: &mut R,
    cancel: &AtomicBool,
    log: &mut dyn FnMut(Message),
    scanner: &mut crate::scanner::LocalScanner,
) -> Result<Report> {
    config.validate()?;
    store::ensure_private_path(&config.local_folder)?;
    if let Some(old) = store.get_meta::<String>("account_id")? {
        if old != remote.account_id() {
            bail!("Server selected a different account; synchronization blocked");
        }
    } else {
        store.set_meta("account_id", &remote.account_id())?;
    }
    // A scan MUST finish completely before any remote mutation or deletion is planned.
    crate::runtime::check()?;
    scanner.set_settle_seconds(config.file_settle_seconds);
    let mut local = scanner.scan(config.max_file_bytes)?;
    let cache = store.get_meta::<Snapshot>("remote_snapshot")?;
    let mut snapshot = remote.snapshot(cache.as_ref())?;
    if cache.as_ref().is_none_or(|old| old.state != snapshot.state) {
        store.set_meta("remote_snapshot", &snapshot)?;
    }
    let remote_folder = config.remote_folder_path()?;
    let root_id = Some(match store.get_meta::<String>("remote_root_id")? {
        Some(id) => {
            if !snapshot
                .nodes
                .iter()
                .any(|n| n.id == id && n.kind == Kind::Directory)
            {
                bail!("Remote sync folder is missing; refusing to recreate it or propagate deletions.");
            }
            id
        }
        None => {
            let node = ensure_folder_path(remote, &mut snapshot, &remote_folder)?;
            store.set_meta("remote_root_id", &node.id)?;
            node.id
        }
    });
    let mut tree = jmap::scoped_tree(&snapshot, root_id.as_deref())?;
    let mut base = store.baselines()?;
    let deferred = scanner.deferred_roots();
    let allowed = |path: &str| {
        !deferred
            .iter()
            .any(|root| path == root || path.starts_with(&format!("{root}/")))
    };
    local.retain(|path, _| allowed(path));
    tree.retain(|path, _| allowed(path));
    base.retain(|path, _| allowed(path));
    if reconcile_rename(
        config,
        store,
        remote,
        &local,
        &tree,
        &base,
        &mut snapshot.state,
        log,
    )? {
        return Ok(Report {
            renamed: 1,
            needs_rescan: true,
            ..Report::default()
        });
    }
    let actions = planner::plan(&local, &tree, &base, config)?;
    let mut report = Report {
        deferred: deferred.len(),
        ..Report::default()
    };
    if !deferred.is_empty() {
        log(Message::new("log.deferred").with("count", deferred.len()));
    }
    let mut trash_id: Option<String> = None;
    for action in actions {
        crate::runtime::check()?;
        if cancel.load(Ordering::Relaxed) {
            bail!("Synchronization paused");
        }
        let path = action.path().to_owned();
        let expected = local.get(&path);
        if matches!(action, Action::Blocked(_)) {
            report.blocked += 1;
            continue;
        }
        if matches!(action, Action::Forget(_)) {
            store.forget(&path)?;
            continue;
        }
        if matches!(action, Action::Accept(_)) {
            let l = expected.context("Accept requires a local entry")?;
            let node = tree.get(&path).context("Accept requires a remote entry")?;
            // No state transition is needed for an identical baseline. Rehashing and
            // writing every accepted file would defeat the event-invalidated cache.
            if base.get(&path).is_some_and(|b| {
                &b.local == l && b.remote_id == node.id && b.remote_blob == node.blob_id
            }) {
                continue;
            }
            disk::verify_local(&config.local_folder, &path, Some(l), config.max_file_bytes)?;
            store.record(&path, l, node)?;
            continue;
        }
        if expected.is_some_and(|entry| entry.kind == Kind::File) {
            let target = disk::checked_path(&config.local_folder, &path)?;
            if disk::open_snapshot(&target).is_err() {
                report.deferred += 1;
                log(Message::new("log.busy").with("path", &path));
                continue;
            }
        }
        let op = store.begin(&action)?;
        let completed = match &action {
            Action::Upload(_) => "log.uploaded",
            Action::Download(_) => "log.downloaded",
            Action::DeleteLocal(_) | Action::DeleteRemote(_) => "log.recycled",
            _ => "log.completed",
        };
        log(Message::new(action_label(&action)).with("path", &path));
        match action {
            Action::CreateLocalDir(_) => {
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.max_file_bytes)?;
                fs::create_dir(disk::checked_path(&config.local_folder, &path)?)?;
                let local = LocalEntry {
                    kind: Kind::Directory,
                    hash: "directory".into(),
                    size: 0,
                };
                store.record(&path, &local, &tree[&path])?;
                report.downloaded += 1;
            }
            Action::CreateRemoteDir(_) => {
                disk::verify_local(&config.local_folder, &path, expected, config.max_file_bytes)?;
                let (parent, name) = parent_and_name(&tree, root_id.as_deref(), &path)?;
                let node = remote.create(
                    name,
                    parent.as_deref(),
                    Kind::Directory,
                    None,
                    0,
                    &mut snapshot.state,
                )?;
                store.record(&path, expected.context("Directory is missing")?, &node)?;
                tree.insert(path.clone(), node);
                report.uploaded += 1;
            }
            Action::Upload(_) => {
                let expected = expected.context("Upload requires a local file")?;
                disk::verify_local(
                    &config.local_folder,
                    &path,
                    Some(expected),
                    config.max_file_bytes,
                )?;
                let source = disk::checked_path(&config.local_folder, &path)?;
                let staged = tempfile::NamedTempFile::new_in(
                    config
                        .local_folder
                        .join(disk::PRIVATE_DIR)
                        .join("transfers"),
                )?
                .into_temp_path();
                // Close the creating write handle before a deny-write snapshot is opened.
                disk::stage_snapshot(&source, &staged)?;
                if &disk::hash_file(&staged, config.max_file_bytes)? != expected {
                    bail!("File changed while preparing upload: {path}");
                }
                let blob = remote.upload(&staged, expected.size)?;
                disk::verify_local(
                    &config.local_folder,
                    &path,
                    Some(expected),
                    config.max_file_bytes,
                )?;
                let (parent, name) = parent_and_name(&tree, root_id.as_deref(), &path)?;
                let node = if let Some(previous) = tree.get(&path) {
                    let trash = ensure_trash(
                        remote,
                        store,
                        root_id.as_deref(),
                        &snapshot.nodes,
                        &mut trash_id,
                        &mut snapshot.state,
                    )?;
                    let backup_name = format!(
                        "{}-{}",
                        uuid::Uuid::new_v4().simple(),
                        previous.name.chars().take(100).collect::<String>()
                    );
                    remote.create(
                        &backup_name,
                        Some(&trash),
                        previous.kind,
                        previous.blob_id.as_deref(),
                        previous.size,
                        &mut snapshot.state,
                    )?;
                    remote.update(
                        previous,
                        name,
                        parent.as_deref(),
                        Some((&blob, expected.size)),
                        &mut snapshot.state,
                    )?
                } else {
                    remote.create(
                        name,
                        parent.as_deref(),
                        Kind::File,
                        Some(&blob),
                        expected.size,
                        &mut snapshot.state,
                    )?
                };
                store.record(&path, expected, &node)?;
                tree.insert(path.clone(), node);
                report.uploaded += 1;
            }
            Action::Download(_) | Action::Reconcile(_) => {
                let node = tree.get(&path).context("Download requires a remote file")?;
                let staged = jmap::transfer_path(&config.local_folder, node);
                remote.download(node, &staged)?;
                let incoming = disk::hash_file(&staged, config.max_file_bytes)?;
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.max_file_bytes)?;
                if expected == Some(&incoming) {
                    fs::remove_file(&staged)?;
                } else {
                    let target = disk::checked_path(&config.local_folder, &path)?;
                    if target.exists() {
                        if matches!(action, Action::Reconcile(_)) {
                            let conflict = disk::conflict_path(&config.local_folder, &path)?;
                            fs::rename(
                                &target,
                                disk::checked_path(&config.local_folder, &conflict)?,
                            )?;
                            log(Message::new("log.preserved").with("path", &conflict));
                            report.conflicts += 1;
                            report.needs_rescan = true;
                        } else {
                            disk::preserve(&config.local_folder, &path)?;
                        }
                    }
                    fs::rename(&staged, &target).context("Atomically install download")?;
                    report.downloaded += 1;
                }
                store.record(&path, &incoming, node)?;
            }
            Action::Conflict(_) => {
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.max_file_bytes)?;
                let conflict = disk::conflict_path(&config.local_folder, &path)?;
                fs::rename(
                    disk::checked_path(&config.local_folder, &path)?,
                    disk::checked_path(&config.local_folder, &conflict)?,
                )?;
                store.forget_tree(&path)?;
                log(Message::new("log.type_conflict").with("path", &conflict));
                report.conflicts += 1;
                report.needs_rescan = true;
                store.done(&op)?;
                return Ok(report);
            }
            Action::DeleteLocal(_) => {
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.max_file_bytes)?;
                let target = disk::checked_path(&config.local_folder, &path)?;
                if target.is_dir() && fs::read_dir(&target)?.next().is_some() {
                    bail!("Directory is not empty; preserving new local files: {path}");
                }
                disk::preserve(&config.local_folder, &path)?;
                store.forget(&path)?;
                report.recycled += 1;
            }
            Action::DeleteRemote(_) => {
                disk::verify_local(&config.local_folder, &path, None, config.max_file_bytes)?;
                let trash = ensure_trash(
                    remote,
                    store,
                    root_id.as_deref(),
                    &snapshot.nodes,
                    &mut trash_id,
                    &mut snapshot.state,
                )?;
                let node = &tree[&path];
                let name = format!(
                    "{}-{}",
                    uuid::Uuid::new_v4().simple(),
                    node.name.chars().take(100).collect::<String>()
                );
                remote.update(node, &name, Some(&trash), None, &mut snapshot.state)?;
                store.forget(&path)?;
                tree.remove(&path);
                report.recycled += 1;
            }
            Action::MoveLocal { .. }
            | Action::MoveRemote { .. }
            | Action::Accept(_)
            | Action::Forget(_)
            | Action::Blocked(_) => unreachable!(),
        }
        store.done(&op)?;
        log(Message::new(completed).with("path", &path));
    }
    Ok(report)
}

pub fn find_folder_path(snapshot: &Snapshot, path: &str) -> Result<Option<String>> {
    Config::validate_remote_path(path)?;
    let mut parent = None;
    for name in path.split('/') {
        let matches: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id == parent && disk::folded(&node.name) == disk::folded(name)
            })
            .collect();
        match matches.as_slice() {
            [node] if node.kind == Kind::Directory => parent = Some(node.id.clone()),
            [] => return Ok(None),
            _ => bail!("Remote sync folder is ambiguous or is not a directory"),
        }
    }
    Ok(parent)
}

fn ensure_folder_path<R: Remote>(
    remote: &mut R,
    snapshot: &mut Snapshot,
    path: &str,
) -> Result<Node> {
    Config::validate_remote_path(path)?;
    let mut parent = None;
    let mut result = None;
    for name in path.split('/') {
        let matches: Vec<_> = snapshot
            .nodes
            .iter()
            .filter(|node| {
                node.parent_id == parent && disk::folded(&node.name) == disk::folded(name)
            })
            .cloned()
            .collect();
        let node = match matches.as_slice() {
            [node] if node.kind == Kind::Directory => node.clone(),
            [] => {
                let node = remote.create(
                    name,
                    parent.as_deref(),
                    Kind::Directory,
                    None,
                    0,
                    &mut snapshot.state,
                )?;
                snapshot.nodes.push(node.clone());
                node
            }
            _ => bail!("Remote sync folder is ambiguous or is not a directory"),
        };
        parent = Some(node.id.clone());
        result = Some(node);
    }
    result.context("Remote folder path is empty")
}

fn check_state<R: Remote>(remote: &R, expected: &str) -> Result<()> {
    if remote.current_state()? != expected {
        bail!("Server changed; discard this plan and synchronize again");
    }
    Ok(())
}

fn ensure_trash<R: Remote>(
    remote: &mut R,
    store: &Store,
    root: Option<&str>,
    nodes: &[Node],
    cached: &mut Option<String>,
    state: &mut String,
) -> Result<String> {
    if let Some(id) = cached {
        check_state(remote, state)?;
        return Ok(id.clone());
    }
    let matches: Vec<_> = nodes
        .iter()
        .filter(|n| n.parent_id.as_deref() == root && disk::folded(&n.name) == disk::REMOTE_TRASH)
        .collect();
    let id = match matches.as_slice() {
        [node] if node.kind == Kind::Directory => {
            check_state(remote, state)?;
            node.id.clone()
        }
        [] => {
            remote
                .create(disk::REMOTE_TRASH, root, Kind::Directory, None, 0, state)?
                .id
        }
        _ => bail!("Invalid remote recovery storage"),
    };
    store.set_meta("trash_id", &id)?;
    *cached = Some(id.clone());
    Ok(id)
}

fn parent_and_name<'a>(
    tree: &RemoteTree,
    root: Option<&str>,
    path: &'a str,
) -> Result<(Option<String>, &'a str)> {
    match path.rsplit_once('/') {
        Some((parent, name)) => Ok((
            Some(
                tree.get(parent)
                    .filter(|n| n.kind == Kind::Directory)
                    .context("Remote parent directory is missing")?
                    .id
                    .clone(),
            ),
            name,
        )),
        None => Ok((root.map(str::to_owned), path)),
    }
}

// Keep all three snapshots explicit at this reconciliation boundary.
#[allow(clippy::too_many_arguments)]
fn reconcile_rename<R: Remote>(
    config: &Config,
    store: &mut Store,
    remote: &mut R,
    local: &LocalTree,
    tree: &RemoteTree,
    base: &Baselines,
    state: &mut String,
    log: &mut dyn FnMut(Message),
) -> Result<bool> {
    let by_id: HashMap<_, _> = tree.iter().map(|(p, n)| (n.id.as_str(), (p, n))).collect();
    let mut entries: Vec<_> = base.iter().collect();
    entries.sort_by_key(|(p, _)| p.split('/').count());
    for (old, b) in &entries {
        if let Some((new, _node)) = by_id.get(b.remote_id.as_str()) {
            if *old != *new && local.contains_key(*old) && !local.contains_key(*new) {
                let destination = disk::checked_path(&config.local_folder, new)?;
                if !destination.parent().is_some_and(Path::is_dir) {
                    continue;
                }
                check_state(remote, state)?;
                disk::verify_local(
                    &config.local_folder,
                    old,
                    local.get(*old),
                    config.max_file_bytes,
                )?;
                let op = store.begin(&Action::MoveLocal {
                    from: (*old).clone(),
                    to: (*new).clone(),
                })?;
                fs::rename(disk::checked_path(&config.local_folder, old)?, destination)?;
                store.rename_baseline(old, new)?;
                store.done(&op)?;
                log(Message::new("log.remote_rename")
                    .with("from", old)
                    .with("to", new));
                return Ok(true);
            }
        }
    }
    let root = Some(
        store
            .get_meta::<String>("remote_root_id")?
            .context("Root is missing")?,
    );
    for (old, b) in entries {
        if b.local.kind != Kind::File || local.contains_key(old) {
            continue;
        }
        let Some(node) = tree
            .get(old)
            .filter(|n| n.id == b.remote_id && n.blob_id == b.remote_blob)
        else {
            continue;
        };
        let matches: Vec<_> = local
            .iter()
            .filter(|(p, l)| !base.contains_key(*p) && !tree.contains_key(*p) && *l == &b.local)
            .collect();
        if matches.len() != 1 {
            continue;
        }
        let (new, value) = matches[0];
        let Ok((parent, name)) = parent_and_name(tree, root.as_deref(), new) else {
            continue;
        };
        disk::verify_local(&config.local_folder, old, None, config.max_file_bytes)?;
        disk::verify_local(
            &config.local_folder,
            new,
            Some(value),
            config.max_file_bytes,
        )?;
        let op = store.begin(&Action::MoveRemote {
            from: old.clone(),
            to: new.clone(),
        })?;
        remote.update(node, name, parent.as_deref(), None, state)?;
        store.rename_baseline(old, new)?;
        store.done(&op)?;
        log(Message::new("log.local_rename")
            .with("from", old)
            .with("to", new));
        return Ok(true);
    }
    Ok(false)
}

fn action_label(action: &Action) -> &'static str {
    match action {
        Action::Upload(_) => "log.upload",
        Action::Download(_) => "log.download",
        Action::Reconcile(_) => "log.reconcile",
        Action::Conflict(_) => "log.conflict",
        Action::CreateLocalDir(_) | Action::CreateRemoteDir(_) => "log.mkdir",
        Action::DeleteLocal(_) | Action::DeleteRemote(_) => "log.recycle",
        _ => "log.sync",
    }
}
