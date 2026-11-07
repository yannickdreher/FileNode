use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FILE_CAPABILITY: &str = "urn:ietf:params:jmap:filenode";
pub const CORE_CAPABILITY: &str = "urn:ietf:params:jmap:core";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEntry {
    pub kind: Kind,
    pub hash: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub blob_id: Option<String>,
    pub kind: Kind,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: String,
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Baseline {
    pub remote_id: String,
    pub remote_blob: Option<String>,
    pub local: LocalEntry,
}

pub type LocalTree = BTreeMap<String, LocalEntry>;
pub type RemoteTree = BTreeMap<String, Node>;
pub type Baselines = BTreeMap<String, Baseline>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    MoveLocal { from: String, to: String },
    MoveRemote { from: String, to: String },
    Upload(String),
    Download(String),
    Reconcile(String),
    CreateLocalDir(String),
    CreateRemoteDir(String),
    DeleteLocal(String),
    DeleteRemote(String),
    Accept(String),
    Forget(String),
    Conflict(String),
    Blocked(String),
}

impl Action {
    pub fn path(&self) -> &str {
        match self {
            Self::MoveLocal { from, .. } | Self::MoveRemote { from, .. } => from,
            Self::Upload(p)
            | Self::Download(p)
            | Self::Reconcile(p)
            | Self::CreateLocalDir(p)
            | Self::CreateRemoteDir(p)
            | Self::DeleteLocal(p)
            | Self::DeleteRemote(p)
            | Self::Accept(p)
            | Self::Forget(p)
            | Self::Conflict(p)
            | Self::Blocked(p) => p,
        }
    }
    pub fn is_delete(&self) -> bool {
        matches!(self, Self::DeleteLocal(_) | Self::DeleteRemote(_))
    }
}
