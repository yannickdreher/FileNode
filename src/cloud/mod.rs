//! Windows Cloud Files integration. A placeholder's identity is a committed
//! snapshot, never evidence that a local edit has reached the server.
#[cfg(any(windows, test))]
use crate::model::Kind;
use crate::model::{LocalEntry, Node};
#[cfg(any(windows, test))]
use anyhow::bail;
use anyhow::Result;
#[cfg(any(windows, test))]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(not(windows))]
use std::path::Path;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(any(windows, test))]
const MAGIC: &str = "FileNode.CloudFiles/1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg(any(windows, test))]
struct Identity {
    magic: String,
    root: String,
    node: Node,
    local: LocalEntry,
}
#[cfg(any(windows, test))]
impl Identity {
    fn new(root: &str, node: &Node, local: Option<&LocalEntry>) -> Self {
        Self {
            magic: MAGIC.into(),
            root: root.into(),
            node: node.clone(),
            local: local.cloned().unwrap_or_else(|| LocalEntry {
                kind: node.kind,
                size: node.size,
                // Stable across hydration, eviction and restarts. Only an actual
                // local edit is hashed. This prevents downloading then uploading
                // the very same version again.
                hash: if node.kind == Kind::Directory {
                    "directory".into()
                } else {
                    format!(
                        "cloud:{}",
                        hex::encode(Sha256::digest(
                            node.blob_id.as_deref().unwrap_or("").as_bytes()
                        ))
                    )
                },
            }),
        }
    }
    fn encode(&self) -> Result<Vec<u8>> {
        if self.node.size > i64::MAX as u64 {
            bail!("Cloud file size exceeds the Windows limit");
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 4096 {
            bail!("Cloud file identity exceeds the Windows limit");
        }
        Ok(bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 4096 {
            bail!("Invalid cloud file identity length");
        }
        let value: Self = serde_json::from_slice(bytes)?;
        if value.magic != MAGIC
            || value.root.len() != 64
            || value.local.kind != value.node.kind
            || value.local.size != value.node.size
            || value.node.size > i64::MAX as u64
            || (value.node.kind == Kind::File && value.node.blob_id.is_none())
        {
            bail!("Invalid FileNode cloud file identity");
        }
        Ok(value)
    }
}

pub fn root_key(config: &crate::config::Config) -> Result<String> {
    let identity = serde_json::to_vec(&(
        config.server.trim_end_matches('/'),
        &config.username,
        &config.account_id,
        config.local_folder.canonicalize()?,
        config.remote_folder_path()?,
    ))?;
    Ok(hex::encode(Sha256::digest(identity)))
}

#[cfg(not(windows))]
pub fn committed_entry(_: &Path) -> Result<Option<LocalEntry>> {
    Ok(None)
}
#[cfg(not(windows))]
pub fn is_owned(_: &Path) -> Result<bool> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn matches_node(_: &Path, _: &Node, _: &LocalEntry) -> Result<bool> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn is_registered(_: &crate::config::Config) -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hydration_does_not_change_committed_identity() {
        let node = Node {
            id: "id".into(),
            name: "a".into(),
            parent_id: None,
            blob_id: Some("blob".into()),
            kind: Kind::File,
            size: 17,
            role: None,
        };
        let original = Identity::new(&"a".repeat(64), &node, None);
        let decoded = Identity::decode(&original.encode().unwrap()).unwrap();
        assert_eq!(decoded.local, original.local);
        assert!(Identity::decode(b"foreign provider").is_err());
        assert!(Identity::decode(&vec![b' '; 4097]).is_err());
    }
}
