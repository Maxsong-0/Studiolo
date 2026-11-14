//! stdin 的虚拟路径在根下只有一个普通文件；多出任何节点或第二个 header 都拒绝。
use super::plan::Marker;
use crate::{ProcessError, Result, runner::inspect::snapshot::Snapshot};
use serde::Deserialize;
use studiolo_backup::worker::restic::SnapshotRef;

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
    mode: u32,
    size: Option<u64>,
}
pub(crate) fn verify(
    bytes: &[u8],
    marker: &Marker,
    snapshot: &SnapshotRef,
    length: u64,
) -> Result<()> {
    let mut header = false;
    let mut file = false;
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        match serde_json::from_slice(line).map_err(|_| ProcessError::Protocol)? {
            Record::Snapshot(value) => {
                if header || value.id != *snapshot {
                    return Err(ProcessError::Protocol);
                }
                value.check_named(&marker.tag, &marker.path)?;
                header = true;
            }
            Record::Node(node) => {
                if !header
                    || file
                    || node.path != marker.path.to_string_lossy()
                    || Some(node.name.as_str()) != marker.path.file_name().and_then(|n| n.to_str())
                    || node.kind != "file"
                    || node.size != Some(length)
                    || node.mode != 0o644
                {
                    return Err(ProcessError::Protocol);
                }
                file = true;
            }
        }
    }
    if !header || !file {
        return Err(ProcessError::Protocol);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/support/final_listing.rs"]
mod tests;
