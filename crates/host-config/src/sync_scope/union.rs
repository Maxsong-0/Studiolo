//! 判定核只消费有界元数据；不扫描磁盘、不读取正文、不提交或生成删除动作。

use super::{
    ApprovedSyncUnion, SyncBinding, SyncFileKind, SyncPathEvidence, SyncScope, SyncScopeError,
    paths,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_PATHS: usize = 4096;
const MAX_PATH_BYTES: usize = 1024 * 1024;
const MAX_OBSERVED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub(super) fn approve(
    scope: &SyncScope,
    binding: &SyncBinding,
    baseline: &[SyncPathEvidence],
    canonical: &[SyncPathEvidence],
    mirror: &[SyncPathEvidence],
) -> Result<ApprovedSyncUnion, SyncScopeError> {
    if &scope.binding != binding {
        return Err(SyncScopeError::BindingMismatch);
    }
    let mut union = BTreeSet::new();
    let mut groups = Vec::new();
    let mut bytes = 0_u64;
    let mut path_bytes = 0_usize;
    let mut index = paths::Index::default();
    for path in scope
        .document
        .whole_folders
        .iter()
        .map(|rule| &rule.prefix)
        .chain(scope.document.whole_files.iter().map(|rule| &rule.path))
        .chain(&scope.protected)
        .chain(&scope.generated)
    {
        index.add(path)?;
    }
    for observations in [baseline, canonical, mirror] {
        if observations.len() > MAX_PATHS {
            return Err(SyncScopeError::Budget);
        }
        let mut seen = BTreeSet::new();
        for item in observations {
            index.add(&item.path)?;
            if !seen.insert(item.path.clone()) {
                return Err(SyncScopeError::Collision);
            }
            if union.insert(item.path.clone()) {
                path_bytes = path_bytes
                    .checked_add(item.path.as_str().len())
                    .ok_or(SyncScopeError::Budget)?;
                if union.len() > MAX_PATHS || path_bytes > MAX_PATH_BYTES {
                    return Err(SyncScopeError::Budget);
                }
            }
            // Missing 仍需要相同批准；基线旧路径不得被漏掉后冒充删除。
            let policy = scope.policy(&item.path)?;
            if let Some(content) = item.content {
                let allowed = match content.kind {
                    SyncFileKind::Markdown => policy.markdown,
                    SyncFileKind::Attachment(kind) => policy.attachment_types.contains(&kind),
                };
                if !allowed {
                    return Err(SyncScopeError::ScopeChangeRequired);
                }
                if content.bytes > policy.max_bytes {
                    return Err(SyncScopeError::Budget);
                }
                bytes = bytes
                    .checked_add(content.bytes)
                    .ok_or(SyncScopeError::Budget)?;
                if bytes > MAX_OBSERVED_BYTES {
                    return Err(SyncScopeError::Budget);
                }
            }
        }
        groups.push(seen);
    }
    paths::files(&union)?;
    if groups[1] != union || groups[2] != union {
        return Err(SyncScopeError::Incomplete);
    }
    let mut approved = BTreeMap::new();
    for path in union {
        let policy = scope.policy(&path)?;
        approved.insert(path, policy);
    }
    Ok(ApprovedSyncUnion {
        binding: scope.binding.clone(),
        paths: approved,
    })
}
