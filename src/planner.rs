use crate::{config::Config, model::*};
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub fn plan(
    local: &LocalTree,
    remote: &RemoteTree,
    base: &Baselines,
    config: &Config,
) -> Result<Vec<Action>> {
    let paths: BTreeSet<_> = local
        .keys()
        .chain(remote.keys())
        .chain(base.keys())
        .cloned()
        .collect();
    let mut actions = Vec::new();
    let mut blocked_trees: Vec<String> = Vec::new();
    for path in paths {
        if blocked_trees
            .iter()
            .any(|p| path.starts_with(&format!("{p}/")))
        {
            continue;
        }
        let (l, r, b) = (local.get(&path), remote.get(&path), base.get(&path));
        let action = match (l, r, b) {
            (None, None, _) => Action::Forget(path.clone()),
            (Some(l), None, None) => upload_or_mkdir(&path, l.kind),
            (None, Some(r), None) => download_or_mkdir(&path, r.kind),
            (Some(l), Some(r), _) if l.kind != r.kind => Action::Conflict(path.clone()),
            (Some(l), Some(_), _) if l.kind == Kind::Directory => Action::Accept(path.clone()),
            (Some(_), Some(_), None) => Action::Reconcile(path.clone()),
            (Some(l), Some(r), Some(b)) => {
                let lc = l != &b.local;
                let rc =
                    r.id != b.remote_id || r.blob_id != b.remote_blob || r.kind != b.local.kind;
                match (lc, rc) {
                    (false, false) => Action::Accept(path.clone()),
                    (true, false) => upload_or_mkdir(&path, l.kind),
                    (false, true) => download_or_mkdir(&path, r.kind),
                    (true, true) if l.kind == Kind::Directory => Action::Accept(path.clone()),
                    (true, true) => Action::Reconcile(path.clone()),
                }
            }
            (Some(l), None, Some(b)) => {
                let dirty_descendant = l.kind == Kind::Directory
                    && local.iter().any(|(p, child)| {
                        p.starts_with(&format!("{path}/"))
                            && base.get(p).map(|b| &b.local) != Some(child)
                    });
                if l != &b.local || dirty_descendant {
                    upload_or_mkdir(&path, l.kind)
                } else if config.propagate_deletes {
                    Action::DeleteLocal(path.clone())
                } else {
                    Action::Blocked(path.clone())
                }
            }
            (None, Some(r), Some(b)) => {
                let dirty_descendant = r.kind == Kind::Directory
                    && remote.iter().any(|(p, child)| {
                        p.starts_with(&format!("{path}/"))
                            && base.get(p).map(|b| (&b.remote_id, &b.remote_blob))
                                != Some((&child.id, &child.blob_id))
                    });
                if r.id != b.remote_id || r.blob_id != b.remote_blob || dirty_descendant {
                    download_or_mkdir(&path, r.kind)
                } else if config.propagate_deletes {
                    Action::DeleteRemote(path.clone())
                } else {
                    Action::Blocked(path.clone())
                }
            }
        };
        if matches!(action, Action::Blocked(_) | Action::Conflict(_)) {
            blocked_trees.push(path.clone());
        }
        actions.push(action);
    }
    let count = actions.iter().filter(|a| a.is_delete()).count();
    let fraction_limit = (base.len() * config.max_delete_percent / 100).max(3);
    if count > config.max_delete_count || count > fraction_limit {
        bail!("Deletion protection: {count} deletions planned. Limits: {} absolute, {} relative. Check the cause before proceeding; no files have been changed.", config.max_delete_count, fraction_limit);
    }
    // One remote directory move preserves the entire deleted tree in the trash.
    // Keep the mass-deletion guard above based on the full affected entry count.
    let deleted_directories: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            Action::DeleteRemote(path)
                if remote
                    .get(path)
                    .is_some_and(|node| node.kind == Kind::Directory) =>
            {
                Some(path.clone())
            }
            _ => None,
        })
        .collect();
    actions.retain(|action| !matches!(action, Action::DeleteRemote(path) if deleted_directories.iter().any(|parent| path.starts_with(&format!("{parent}/")))));
    // Create parents first; remove local children first. Accept/forget require no I/O.
    actions.sort_by_key(|a| {
        let depth = a.path().split('/').count();
        let group = match a {
            Action::CreateLocalDir(_) | Action::CreateRemoteDir(_) => 0,
            Action::Accept(_) | Action::Forget(_) => 1,
            Action::DeleteLocal(_) | Action::DeleteRemote(_) => 3,
            _ => 2,
        };
        (
            group,
            if group == 3 {
                usize::MAX - depth
            } else {
                depth
            },
            a.path().to_owned(),
        )
    });
    Ok(actions)
}

fn upload_or_mkdir(p: &str, kind: Kind) -> Action {
    if kind == Kind::Directory {
        Action::CreateRemoteDir(p.into())
    } else {
        Action::Upload(p.into())
    }
}
fn download_or_mkdir(p: &str, kind: Kind) -> Action {
    if kind == Kind::Directory {
        Action::CreateLocalDir(p.into())
    } else {
        Action::Download(p.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(hash: &str) -> LocalEntry {
        LocalEntry {
            kind: Kind::File,
            hash: hash.into(),
            size: 1,
        }
    }
    fn node(blob: &str) -> Node {
        Node {
            id: "id".into(),
            name: "a".into(),
            parent_id: None,
            blob_id: Some(blob.into()),
            kind: Kind::File,
            size: 1,
            role: None,
        }
    }
    fn base() -> Baselines {
        [(
            "a".into(),
            Baseline {
                remote_id: "id".into(),
                remote_blob: Some("old".into()),
                local: file("old"),
            },
        )]
        .into()
    }
    fn cfg() -> Config {
        Config {
            propagate_deletes: true,
            ..Config::default()
        }
    }
    #[test]
    fn three_way_matrix() {
        for (local, remote, expected) in [
            (Some("old"), Some("old"), Action::Accept("a".into())),
            (Some("new"), Some("old"), Action::Upload("a".into())),
            (Some("old"), Some("new"), Action::Download("a".into())),
            (Some("new"), Some("new"), Action::Reconcile("a".into())),
            (None, Some("old"), Action::DeleteRemote("a".into())),
            (Some("old"), None, Action::DeleteLocal("a".into())),
            (None, Some("new"), Action::Download("a".into())),
            (Some("new"), None, Action::Upload("a".into())),
            (None, None, Action::Forget("a".into())),
        ] {
            let l = local
                .map(|h| [("a".into(), file(h))].into())
                .unwrap_or_default();
            let r = remote
                .map(|h| [("a".into(), node(h))].into())
                .unwrap_or_default();
            assert_eq!(plan(&l, &r, &base(), &cfg()).unwrap(), vec![expected]);
        }
    }
    #[test]
    fn initial_sync_never_deletes() {
        let l = [("a".into(), file("a"))].into();
        assert_eq!(
            plan(&l, &RemoteTree::new(), &Baselines::new(), &cfg()).unwrap(),
            vec![Action::Upload("a".into())]
        );
    }
    #[test]
    fn mass_deletion_is_blocked() {
        let mut b = Baselines::new();
        let mut r = RemoteTree::new();
        for i in 0..100 {
            b.insert(i.to_string(), base()["a"].clone());
            r.insert(i.to_string(), node("old"));
        }
        assert!(plan(&LocalTree::new(), &r, &b, &cfg()).is_err());
    }
    #[test]
    fn deletion_propagation_can_be_disabled() {
        let l = [("a".into(), file("old"))].into();
        assert_eq!(
            plan(
                &l,
                &RemoteTree::new(),
                &base(),
                &Config {
                    propagate_deletes: false,
                    ..Config::default()
                }
            )
            .unwrap(),
            vec![Action::Blocked("a".into())]
        );
    }
    proptest::proptest! {
        #[test]
        fn simultaneous_edit_never_deletes(a in "[a-z]{1,20}", b in "[a-z]{1,20}") {
            let l = [("a".into(), file(&a))].into();
            let r = [("a".into(), node(&b))].into();
            let actions = plan(&l, &r, &base(), &cfg()).unwrap();
            proptest::prop_assert!(!actions.iter().any(Action::is_delete));
        }
    }
}
