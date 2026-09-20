//! 全批先解析、限定预算并重验 CAS；这里只产生内存候选。
use super::*;
use crate::{BrokerError, FsVaultBroker, Mutation, Precondition, VaultObject, VaultUuid};

pub(super) fn invalid(reason: &str) -> BrokerError {
    BrokerError::InvalidArtifact(reason.into())
}
pub(super) fn validate(request: &BatchRequest) -> Result<Vec<&BatchChange>, BrokerError> {
    if request.batch_id.is_empty()
        || request.batch_id.len() > 256
        || request.changes.is_empty()
        || request.changes.len() > 32
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut changes: Vec<_> = request.changes.iter().collect();
    changes.sort_by_key(|change| change.path.collision_key());
    let mut bytes = 0usize;
    for change in &changes {
        if change.path.is_protected() {
            return Err(BrokerError::ProtectedPath(change.path.clone()));
        }
        if !change.path.as_str().ends_with(".md") {
            return Err(invalid("批量写回仅接受 Markdown"));
        }
        bytes = bytes
            .checked_add(match &change.edit {
                BatchEdit::Create { content } => content.len(),
                BatchEdit::Patch { patches, .. } => {
                    if patches.is_empty() || patches.len() > 128 {
                        return Err(invalid("补丁数量无效"));
                    }
                    patches
                        .iter()
                        .try_fold(0usize, |sum, p| {
                            sum.checked_add(p.before.len())
                                .and_then(|n| n.checked_add(p.after.len()))
                        })
                        .ok_or(BrokerError::WriterLimitExceeded)?
                }
            })
            .ok_or(BrokerError::WriterLimitExceeded)?;
    }
    if bytes > 4 * 1024 * 1024 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    for (index, left) in changes.iter().enumerate() {
        for right in changes.iter().skip(index + 1) {
            let left_key = left.path.collision_key();
            let right_key = right.path.collision_key();
            if left_key == right_key
                || right_key.starts_with(&format!("{left_key}/"))
                || left_key.starts_with(&format!("{right_key}/"))
            {
                return Err(invalid("批量路径重复或互为祖先"));
            }
            for (a, b) in left
                .path
                .as_str()
                .split('/')
                .zip(right.path.as_str().split('/'))
            {
                if crate::collision_component_key(a) != crate::collision_component_key(b) {
                    break;
                }
                if a != b {
                    return Err(invalid("批量父目录拼写冲突"));
                }
            }
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}
pub(super) fn markdown(text: &str) -> Result<(), BrokerError> {
    if text.contains('\0') {
        return Err(invalid("Markdown 含 NUL"));
    }
    if let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        let mut yaml = String::new();
        let mut closed = false;
        for line in rest.lines() {
            if line == "---" || line == "..." {
                closed = true;
                break;
            }
            yaml.push_str(line);
            yaml.push('\n');
        }
        if !closed {
            return Err(invalid("YAML frontmatter 未闭合"));
        }
        let value: serde_json::Value =
            serde_saphyr::from_str(&yaml).map_err(|_| invalid("YAML frontmatter 无效"))?;
        if !value.is_object() && !value.is_null() {
            return Err(invalid("YAML frontmatter 必须为映射"));
        }
    }
    Ok(())
}
pub(super) fn build(
    uuid: VaultUuid,
    request: &BatchRequest,
    mut read: impl FnMut(&crate::VaultPath) -> Result<Option<VaultObject>, BrokerError>,
) -> Result<(BatchPreview, Vec<Mutation>), BrokerError> {
    let mut diffs = Vec::new();
    let mut mutations = Vec::new();
    let mut bytes = 0usize;
    for change in validate(request)? {
        let object = read(&change.path)?;
        let precondition = match &change.edit {
            BatchEdit::Create { .. } => Precondition::MustNotExist,
            BatchEdit::Patch { expected, .. } => Precondition::Match(*expected),
        };
        let actual = object.as_ref().map(|o| o.revision);
        if !crate::mutation::record::cas_matches(precondition, actual) {
            return Err(crate::mutation::record::conflict(
                change.path.clone(),
                precondition,
                actual,
            ));
        }
        let before = object
            .map(|o| String::from_utf8(o.content).map_err(|_| invalid("原 Markdown 非 UTF-8")))
            .transpose()?;
        let after = match &change.edit {
            BatchEdit::Create { content } => content.clone(),
            BatchEdit::Patch { patches, .. } => {
                let mut text = before.clone().ok_or_else(|| invalid("缺少补丁原文"))?;
                for patch in patches {
                    if patch.before.is_empty() || text.match_indices(&patch.before).count() != 1 {
                        return Err(invalid("补丁原文必须非空且唯一匹配"));
                    }
                    text = text.replacen(&patch.before, &patch.after, 1);
                }
                text
            }
        };
        markdown(&after)?;
        bytes = bytes
            .checked_add(after.len())
            .and_then(|n| n.checked_add(before.as_ref().map_or(0, String::len)))
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if bytes > 4 * 1024 * 1024 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        mutations.push(Mutation {
            path: change.path.clone(),
            content: after.as_bytes().to_vec(),
            precondition,
            idempotency_key: format!("batch:{}:{}", request.batch_id, change.path),
        });
        diffs.push(BatchDiff {
            path: change.path.clone(),
            before,
            after,
        });
    }
    let fingerprint = FsVaultBroker::fingerprint(&("studiolo-batch-v1", uuid, request, &diffs))?;
    Ok((
        BatchPreview {
            batch_id: request.batch_id.clone(),
            fingerprint,
            diffs,
        },
        mutations,
    ))
}
