//! 对完整快照树逐项移除期望项，漏项、多项、重复项或特殊节点都不能成为恢复点。
use super::snapshot::Snapshot;
use crate::{ProcessError, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
use studiolo_backup::{
    checkpoint::staging::SealedInventory,
    worker::{restic::SnapshotRef, state::JobKey},
};

#[derive(Deserialize)]
#[serde(tag = "message_type")]
enum Record {
    #[serde(rename = "snapshot")]
    Snapshot(Snapshot),
    #[serde(rename = "node")]
    Node(Node),
}
#[derive(Deserialize)]
struct Node {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    path: String,
    uid: u32,
    gid: u32,
    mode: u32,
    size: Option<u64>,
}
pub(crate) struct Expected<'a> {
    pub key: &'a JobKey,
    pub snapshot: &'a SnapshotRef,
    pub root: &'a Path,
    pub inventory: &'a SealedInventory,
    pub producer: u32,
    pub group: u32,
}
pub(crate) fn verify(bytes: &[u8], expected: Expected<'_>) -> Result<()> {
    let mut directories = BTreeMap::new();
    let mut files = BTreeMap::new();
    for ancestor in expected
        .root
        .ancestors()
        .filter(|path| *path != Path::new("/"))
    {
        directories.insert(
            ancestor.to_str().ok_or(ProcessError::Source)?.to_owned(),
            ancestor == expected.root,
        );
    }
    for path in expected.inventory.directories() {
        directories.insert(absolute(expected.root, path)?, true);
    }
    for file in expected.inventory.files() {
        files.insert(absolute(expected.root, file.path().as_str())?, file.bytes());
    }
    let mut header = false;
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let record: Record = serde_json::from_slice(line).map_err(|_| ProcessError::Protocol)?;
        match record {
            Record::Snapshot(snapshot) => {
                if header || snapshot.id != *expected.snapshot {
                    return Err(ProcessError::Protocol);
                }
                snapshot.check(expected.key, expected.root)?;
                header = true;
            }
            Record::Node(node) => {
                if !header
                    || Path::new(&node.path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        != Some(node.name.as_str())
                {
                    return Err(ProcessError::Protocol);
                }
                match node.kind.as_str() {
                    "dir" => {
                        let inside = directories
                            .remove(&node.path)
                            .ok_or(ProcessError::Protocol)?;
                        if node.size.is_some()
                            || (inside
                                && (node.mode != ((1 << 31) | 0o550)
                                    || node.uid != expected.producer
                                    || node.gid != expected.group))
                        {
                            return Err(ProcessError::Protocol);
                        }
                    }
                    "file" => {
                        if node.size
                            != Some(files.remove(&node.path).ok_or(ProcessError::Protocol)?)
                            || node.mode != 0o440
                            || node.uid != expected.producer
                            || node.gid != expected.group
                        {
                            return Err(ProcessError::Protocol);
                        }
                    }
                    _ => return Err(ProcessError::Protocol),
                }
            }
        }
    }
    if !header || !directories.is_empty() || !files.is_empty() {
        return Err(ProcessError::Protocol);
    }
    Ok(())
}
fn absolute(root: &Path, relative: &str) -> Result<String> {
    root.join(relative)
        .to_str()
        .map(str::to_owned)
        .ok_or(ProcessError::Source)
}

#[cfg(test)]
#[path = "../../../tests/support/inspect_listing.rs"]
mod tests;
