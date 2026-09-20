//! 历史查询只读索引和内容；游标精确区分错误输入与同路径目录变化。

use super::{HistoryContent, HistoryPage, catalog};
use crate::writer::{directory, disk};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath, mutation, sha256};

pub(super) fn readable(broker: &FsVaultBroker) -> Result<mutation::State, BrokerError> {
    let state = mutation::load(broker)?;
    if broker.writer.uncertain()
        || state.legacy_unresolved
        || state.batch.is_some()
        || crate::batch::barrier::blocked(&broker.root)?
        || state
            .file_operation
            .as_ref()
            .is_some_and(|op| op.blocked_read())
        || state
            .operation
            .as_ref()
            .is_some_and(|op| op.phase != mutation::record::Phase::Prepared)
    {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(state)
}
pub(super) fn unchanged(
    broker: &FsVaultBroker,
    before: &mutation::State,
) -> Result<(), BrokerError> {
    let after = readable(broker)?;
    if serde_json::to_vec(before).map_err(|_| BrokerError::CorruptWriterState)?
        != serde_json::to_vec(&after).map_err(|_| BrokerError::CorruptWriterState)?
    {
        return Err(BrokerError::HistoryChanged);
    }
    Ok(())
}
pub(super) fn checksum(value: &catalog::Catalog) -> Result<String, BrokerError> {
    Ok(sha256(
        &serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?,
    ))
}
/// 仅内部完成/整理过程可绕过索引可见性；仍验证 UUID + 逻辑路径 + 原字节。
pub(super) fn object(
    broker: &FsVaultBroker,
    path: &VaultPath,
    revision: Revision,
    max: u64,
) -> Result<Vec<u8>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let directory = directory(&broker.data, "history")?;
    let cap = max.min(disk::CONTENT_LIMIT as u64) as usize;
    let content = disk::read(&directory, &revision.to_string(), cap)
        .map_err(|error| match error {
            BrokerError::WriterLimitExceeded => BrokerError::ReadLimitExceeded {
                path: path.clone(),
                max_bytes: cap as u64,
            },
            other => other,
        })?
        .ok_or_else(|| BrokerError::NotFound(path.clone()))?;
    if Revision::compute(broker.vault_uuid, path, &content) != revision {
        return Err(BrokerError::CorruptWriterState);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(content)
}
impl FsVaultBroker {
    /// 最新提交优先；旧版本顺序未知，仅给稳定次序，不用 mtime 伪造时间线。
    pub fn list_history(
        &self,
        path: &VaultPath,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<HistoryPage, BrokerError> {
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = readable(self)?;
        let catalog = catalog::load(self, path)?;
        let digest = checksum(&catalog)?;
        let path_hash = sha256(path.as_str().as_bytes());
        let start = cursor
            .map(|value| parse_cursor(value, &path_hash, &digest))
            .transpose()?
            .unwrap_or(0);
        let mut items = catalog.entries.clone();
        items.retain(|item| item.sequence <= state.committed_through);
        items.sort_by(|a, b| {
            b.sequence
                .cmp(&a.sequence)
                .then_with(|| b.entry.authored_at_ms.cmp(&a.entry.authored_at_ms))
                .then_with(|| a.entry.id.cmp(&b.entry.id))
        });
        if start > items.len() {
            return Err(BrokerError::InvalidHistoryCursor);
        }
        let end = start.saturating_add(limit).min(items.len());
        let entries = items[start..end]
            .iter()
            .map(|item| item.entry.clone())
            .collect();
        let next_cursor = (end < items.len()).then(|| format!("h1:{path_hash}:{digest}:{end}"));
        unchanged(self, &state)?;
        if checksum(&catalog::load(self, path)?)? != digest {
            return Err(BrokerError::HistoryChanged);
        }
        Ok(HistoryPage {
            entries,
            next_cursor,
        })
    }
    /// 读取必须仍在该路径可查询历史内，GC 留下的退役对象不会重新成为恢复来源。
    pub fn read_history(
        &self,
        path: &VaultPath,
        revision: Revision,
        max_bytes: u64,
    ) -> Result<HistoryContent, BrokerError> {
        let state = readable(self)?;
        let catalog = catalog::load(self, path)?;
        let digest = checksum(&catalog)?;
        let entries: Vec<_> = catalog
            .entries
            .iter()
            .filter(|item| {
                item.sequence <= state.committed_through && item.entry.revision == revision
            })
            .collect();
        if entries.is_empty() {
            return Err(BrokerError::NotFound(path.clone()));
        }
        let content = object(self, path, revision, max_bytes)?;
        if entries
            .iter()
            .any(|item| item.entry.bytes != content.len() as u64)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        unchanged(self, &state)?;
        if checksum(&catalog::load(self, path)?)? != digest {
            return Err(BrokerError::HistoryChanged);
        }
        Ok(HistoryContent {
            vault_uuid: self.vault_uuid,
            path: path.clone(),
            revision,
            content,
        })
    }
}
fn parse_cursor(value: &str, path: &str, digest: &str) -> Result<usize, BrokerError> {
    if value.len() > 160 {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    let mut fields = value.split(':');
    let (Some("h1"), Some(bound), Some(generation), Some(offset), None) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) else {
        return Err(BrokerError::InvalidHistoryCursor);
    };
    if !disk::hex(bound)
        || !disk::hex(generation)
        || bound != path
        || offset.is_empty()
        || !offset.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    let offset = offset
        .parse::<usize>()
        .map_err(|_| BrokerError::InvalidHistoryCursor)?;
    if generation != digest {
        return Err(BrokerError::HistoryChanged);
    }
    Ok(offset)
}
