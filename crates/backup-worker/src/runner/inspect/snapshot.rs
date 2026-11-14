//! 固定 restic JSON 的真实快照身份；拒绝短 ID、附带排除项、重写来源和重复作业候选。
use crate::{ProcessError, Result};
use serde::Deserialize;
use std::path::Path;
use studiolo_backup::worker::{
    restic::{JobTag, SnapshotRef},
    state::JobKey,
};

#[derive(Deserialize)]
pub(super) struct Snapshot {
    pub id: SnapshotRef,
    tree: SnapshotRef,
    paths: Vec<String>,
    tags: Vec<String>,
    #[serde(default)]
    excludes: Vec<String>,
    original: Option<SnapshotRef>,
}
impl Snapshot {
    pub fn check(&self, key: &JobKey, root: &Path) -> Result<()> {
        let tag = JobTag::for_job(key).map_err(|_| ProcessError::Configuration)?;
        if self.id.as_str().len() != 64
            || self.tree.as_str().len() != 64
            || self.paths.len() != 1
            || self.paths[0] != root.to_str().ok_or(ProcessError::Source)?
            || self.tags.len() != 1
            || self.tags[0] != tag.as_str()
            || !self.excludes.is_empty()
            || self.original.is_some()
        {
            return Err(ProcessError::Protocol);
        }
        Ok(())
    }
}
pub(crate) fn locate(bytes: &[u8], key: &JobKey, root: &Path) -> Result<Option<SnapshotRef>> {
    let mut snapshots: Vec<Snapshot> =
        serde_json::from_slice(bytes).map_err(|_| ProcessError::Protocol)?;
    if snapshots.len() > 1 {
        return Err(ProcessError::Protocol);
    }
    let Some(snapshot) = snapshots.pop() else {
        return Ok(None);
    };
    snapshot.check(key, root)?;
    Ok(Some(snapshot.id))
}

#[cfg(test)]
#[path = "../../../tests/support/inspect_snapshot.rs"]
mod tests;
