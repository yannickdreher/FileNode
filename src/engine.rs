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
    pub limited: usize,
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
    store::ensure_state_directory(&config.private_directory()?)?;
    scanner.connect_cloud(config, remote)?;
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
    scanner.set_exclusions(&config.exclude_patterns)?;
    let upload_limit = remote.upload_limits().effective_size(config);
    let mut local = scanner.scan(upload_limit)?;
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
    let exclusions = crate::exclusions::Exclusions::new(&config.exclude_patterns)?;
    let mut oversized = scanner.oversized().clone();
    let oversized_downloads: Vec<_> = tree
        .iter()
        .filter(|(path, node)| {
            node.kind == Kind::File && node.size > config.file_limit() && !exclusions.matches(path)
        })
        .map(|(path, node)| (path.clone(), node.size))
        .collect();
    for (path, size) in &oversized_downloads {
        oversized.entry(path.clone()).or_insert(*size);
    }
    let protected = exclusions.scope(
        &mut local,
        &mut tree,
        &mut base,
        scanner.omitted_paths().chain(oversized.keys()),
    );
    for (path, size) in scanner.oversized() {
        crate::activity::size_limit(path, *size, upload_limit);
    }
    for (path, size) in oversized_downloads {
        if !scanner.oversized().contains_key(&path) {
            crate::activity::size_limit(&path, size, config.file_limit());
        }
    }
    local.retain(|path, _| !oversized.contains_key(path));
    tree.retain(|path, _| !oversized.contains_key(path));
    base.retain(|path, _| !oversized.contains_key(path));
    let deferred = scanner.deferred_roots();
    for path in scanner.deferred_paths().take(100) {
        crate::activity::notice(path, "check", "waiting");
    }
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
        &protected,
        &mut snapshot.state,
        log,
    )? {
        return Ok(Report {
            renamed: 1,
            needs_rescan: true,
            ..Report::default()
        });
    }
    let actions = planner::plan_protected(&local, &tree, &base, config, &protected)?;
    let mut report = Report {
        deferred: deferred.len(),
        limited: oversized.len(),
        ..Report::default()
    };
    if !deferred.is_empty() {
        log(Message::new("log.deferred").with("count", deferred.len()));
    }
    let mut history_id: Option<String> = None;
    let mut actions = std::collections::VecDeque::from(actions);
    let mut prepared = HashMap::<String, crate::uploads::Prepared>::new();
    while let Some(action) = actions.pop_front() {
        crate::runtime::check()?;
        if cancel.load(Ordering::Relaxed) {
            bail!("Synchronization paused");
        }
        let path = action.path().to_owned();
        let expected = local.get(&path);
        if matches!(action, Action::Blocked(_)) {
            crate::activity::notice(&path, "trash", "blocked");
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
            }) && (!config.files_on_demand
                || crate::cloud::matches_node(
                    &disk::checked_path(&config.local_folder, &path)?,
                    node,
                    l,
                )?)
            {
                continue;
            }
            disk::verify_local(&config.local_folder, &path, Some(l), config.file_limit())?;
            if config.files_on_demand {
                scanner.publish_cloud(&path, node, Some(l))?;
            }
            store.record(&path, l, node)?;
            continue;
        }
        if expected.is_some_and(|entry| entry.kind == Kind::File)
            && crate::cloud::committed_entry(&disk::checked_path(&config.local_folder, &path)?)?
                .is_none()
        {
            let target = disk::checked_path(&config.local_folder, &path)?;
            if disk::open_snapshot(&target).is_err() {
                report.deferred += 1;
                crate::activity::notice(&path, "check", "waiting");
                log(Message::new("log.busy").with("path", &path));
                continue;
            }
        }
        let op = store.begin(&action)?;
        if matches!(action, Action::Upload(_)) && prepared.is_empty() {
            let count = remote.upload_limits().effective_parallelism(config);
            if count > 1 {
                if let Some(uploader) = remote.uploader() {
                    let files = std::iter::once(&action)
                        .chain(
                            actions
                                .iter()
                                .take_while(|next| matches!(next, Action::Upload(_))),
                        )
                        .take(count)
                        .map(|next| (next.path().to_owned(), local[next.path()].clone()))
                        .collect();
                    prepared = crate::uploads::prepare(config, uploader, files);
                }
            }
        }
        let mut prefetched = prepared.remove(&path);
        let operation = match &action {
            Action::Upload(_) => "upload",
            Action::Download(_) if config.files_on_demand => "placeholder",
            Action::Download(_) | Action::Reconcile(_) => "download",
            Action::DeleteLocal(_) | Action::DeleteRemote(_) => "trash",
            Action::Conflict(_) => "compare",
            _ => "folder",
        };
        let bytes = if operation == "upload" {
            expected.map_or(0, |entry| entry.size)
        } else if operation == "download" {
            tree.get(&path).map_or(0, |entry| entry.size)
        } else {
            0
        };
        let completed_bytes = if prefetched
            .as_ref()
            .is_some_and(|item| item.blob.as_ref().is_some_and(Result::is_ok))
        {
            bytes
        } else {
            0
        };
        let mut activity =
            crate::activity::FileActivity::begin_at(&path, operation, bytes, completed_bytes);
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
                disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
                fs::create_dir(disk::checked_path(&config.local_folder, &path)?)?;
                let local = LocalEntry {
                    kind: Kind::Directory,
                    hash: "directory".into(),
                    size: 0,
                };
                store.record(&path, &local, &tree[&path])?;
                if config.files_on_demand {
                    scanner.publish_cloud(&path, &tree[&path], Some(&local))?;
                }
                report.downloaded += 1;
            }
            Action::CreateRemoteDir(_) => {
                disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
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
                if config.files_on_demand {
                    scanner.publish_cloud(&path, &node, expected)?;
                }
                tree.insert(path.clone(), node);
                report.uploaded += 1;
            }
            Action::Upload(_) => {
                let expected = expected.context("Upload requires a local file")?;
                disk::verify_local(
                    &config.local_folder,
                    &path,
                    Some(expected),
                    config.file_limit(),
                )?;
                let blob = if let Some(item) = &mut prefetched {
                    item.blob
                        .take()
                        .context("Prepared upload was already consumed")??
                } else {
                    let staged = crate::uploads::stage(config, &path, expected)?;
                    remote.upload(&staged, expected.size)?
                };
                disk::verify_local(
                    &config.local_folder,
                    &path,
                    Some(expected),
                    config.file_limit(),
                )?;
                let (parent, name) = parent_and_name(&tree, root_id.as_deref(), &path)?;
                let node = if let Some(previous) = tree.get(&path) {
                    let trash = ensure_version_history(
                        remote,
                        store,
                        root_id.as_deref(),
                        &snapshot.nodes,
                        &mut history_id,
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
                if config.files_on_demand {
                    scanner.publish_cloud(&path, &node, Some(expected))?;
                }
                tree.insert(path.clone(), node);
                report.uploaded += 1;
            }
            Action::Download(_) | Action::Reconcile(_) => {
                let node = tree.get(&path).context("Download requires a remote file")?;
                if config.files_on_demand && !matches!(action, Action::Reconcile(_)) {
                    check_state(remote, &snapshot.state)?;
                    disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
                    let target = disk::checked_path(&config.local_folder, &path)?;
                    if target.exists() && !crate::cloud::is_owned(&target)? {
                        disk::preserve_in(
                            &config.local_folder,
                            &path,
                            &config.private_directory()?,
                        )?;
                    }
                    let incoming = scanner.publish_cloud(&path, node, None)?;
                    store.record(&path, &incoming, node)?;
                    store.done(&op)?;
                    activity.finish("synced");
                    report.downloaded += 1;
                    continue;
                }
                let staged = jmap::transfer_path(&config.local_folder, node);
                let staged = if config.files_on_demand {
                    config
                        .private_directory()?
                        .join("transfers")
                        .join(staged.file_name().context("Transfer name")?)
                } else {
                    staged
                };
                remote.download(node, &staged)?;
                let incoming = disk::hash_file(&staged, config.file_limit())?;
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
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
                            disk::preserve_in(
                                &config.local_folder,
                                &path,
                                &config.private_directory()?,
                            )?;
                        }
                    }
                    disk::install_download(&config.local_folder, &staged, &target)?;
                    report.downloaded += 1;
                }
                store.record(&path, &incoming, node)?;
                if config.files_on_demand {
                    scanner.publish_cloud(&path, node, Some(&incoming))?;
                }
            }
            Action::Conflict(_) => {
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
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
                activity.finish("warning");
                return Ok(report);
            }
            Action::DeleteLocal(_) => {
                check_state(remote, &snapshot.state)?;
                disk::verify_local(&config.local_folder, &path, expected, config.file_limit())?;
                let target = disk::checked_path(&config.local_folder, &path)?;
                if target.is_dir() && fs::read_dir(&target)?.next().is_some() {
                    bail!("Directory is not empty; preserving new local files: {path}");
                }
                if config.files_on_demand && crate::cloud::committed_entry(&target)?.is_some() {
                    if target.is_dir() {
                        fs::remove_dir(&target)?;
                    } else {
                        fs::remove_file(&target)?;
                    }
                } else {
                    disk::preserve_in(&config.local_folder, &path, &config.private_directory()?)?;
                }
                store.forget(&path)?;
                report.recycled += 1;
            }
            Action::DeleteRemote(_) => {
                disk::verify_local(&config.local_folder, &path, None, config.file_limit())?;
                let trash = ensure_server_trash(remote, &mut snapshot)?;
                let node = &tree[&path];
                // Bulwark-compatible batch names retain the original parent for Restore.
                let parent = node.parent_id.as_deref().unwrap_or("");
                if !parent
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                {
                    bail!("Parent ID cannot be represented in the interoperable trash format");
                }
                let newest = snapshot
                    .nodes
                    .iter()
                    .filter(|entry| entry.parent_id.as_deref() == Some(&trash))
                    .filter_map(|entry| entry.name.split_once('~')?.0.parse::<i64>().ok())
                    .max()
                    .unwrap_or(0);
                let timestamp = chrono::Utc::now()
                    .timestamp_millis()
                    .max(newest.saturating_add(1));
                let batch = remote.create(
                    &format!("{timestamp}~{parent}"),
                    Some(&trash),
                    Kind::Directory,
                    None,
                    0,
                    &mut snapshot.state,
                )?;
                let batch_id = batch.id.clone();
                snapshot.nodes.push(batch);
                let moved =
                    remote.update(node, &node.name, Some(&batch_id), None, &mut snapshot.state)?;
                if let Some(previous) = snapshot.nodes.iter_mut().find(|entry| entry.id == moved.id)
                {
                    *previous = moved;
                }
                store.forget_tree(&path)?;
                tree.retain(|entry, _| entry != &path && !entry.starts_with(&format!("{path}/")));
                report.recycled += 1;
            }
            Action::MoveLocal { .. }
            | Action::MoveRemote { .. }
            | Action::Accept(_)
            | Action::Forget(_)
            | Action::Blocked(_) => unreachable!(),
        }
        store.done(&op)?;
        if let Some(item) = &mut prefetched {
            item.committed = true;
        }
        activity.finish("synced");
        log(Message::new(completed).with("path", &path));
    }
    scanner.cloud_availability()?;
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

fn ensure_server_trash<R: Remote>(remote: &mut R, snapshot: &mut Snapshot) -> Result<String> {
    let matches: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|node| {
            node.role.as_deref() == Some("trash")
                || (node.parent_id.is_none() && node.name == ".Trash")
        })
        .collect();
    match matches.as_slice() {
        [node] if node.kind == Kind::Directory => {
            check_state(remote, &snapshot.state)?;
            Ok(node.id.clone())
        }
        [] => {
            let parent = snapshot
                .nodes
                .iter()
                .find(|node| node.kind == Kind::Directory && node.role.as_deref() == Some("root"))
                .map(|node| node.id.clone());
            let node = remote.create_trash(parent.as_deref(), &mut snapshot.state)?;
            if node.kind != Kind::Directory
                || (node.role.as_deref() != Some("trash")
                    && !(node.parent_id.is_none() && node.name == ".Trash"))
            {
                bail!("Server did not create a valid trash directory");
            }
            let id = node.id.clone();
            snapshot.nodes.push(node);
            Ok(id)
        }
        _ => bail!("Server trash role is ambiguous or invalid"),
    }
}

// Private overwritten-version storage is distinct from the account's trash role.
fn ensure_version_history<R: Remote>(
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
        .filter(|n| n.parent_id.as_deref() == root && disk::folded(&n.name) == disk::REMOTE_HISTORY)
        .collect();
    let id = match matches.as_slice() {
        [node] if node.kind == Kind::Directory => {
            check_state(remote, state)?;
            node.id.clone()
        }
        [] => {
            remote
                .create(disk::REMOTE_HISTORY, root, Kind::Directory, None, 0, state)?
                .id
        }
        _ => bail!("Invalid remote recovery storage"),
    };
    store.set_meta("history_id", &id)?;
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
    protected: &std::collections::HashSet<String>,
    state: &mut String,
    log: &mut dyn FnMut(Message),
) -> Result<bool> {
    let by_id: HashMap<_, _> = tree.iter().map(|(p, n)| (n.id.as_str(), (p, n))).collect();
    let mut entries: Vec<_> = base.iter().collect();
    entries.sort_by_key(|(p, _)| p.split('/').count());
    for (old, b) in &entries {
        if protected.contains(*old) {
            continue;
        }
        if let Some((new, _node)) = by_id.get(b.remote_id.as_str()) {
            if protected.contains(*new) {
                continue;
            }
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
                    config.file_limit(),
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
                crate::activity::renamed(old, new);
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
        disk::verify_local(&config.local_folder, old, None, config.file_limit())?;
        disk::verify_local(&config.local_folder, new, Some(value), config.file_limit())?;
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
        crate::activity::renamed(old, new);
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
