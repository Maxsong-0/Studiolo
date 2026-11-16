//! 逐行归属用的版本链：和历史列表同一套可见性与排序，另外标出哪一版是「创建」。
use super::{HistoryLineage, HistoryLineageEntry, catalog, query};
use crate::{BrokerError, FsVaultBroker, Precondition, VaultPath};

/// 只从原请求或原批次终态认「创建」，不看条目里的副本，也不看磁盘时间。
fn created(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &catalog::Item,
) -> Result<bool, BrokerError> {
    if let Some(reference) = &item.recording {
        let envelope = crate::recorded::store::load(broker, reference)?;
        return Ok(
            item.entry.id == crate::sha256(format!("{}:new", envelope.id).as_bytes())
                && envelope.precondition == Precondition::MustNotExist,
        );
    }
    if let Some(reference) = &item.batch {
        let (active, _) = crate::batch::record::history_source(
            broker,
            &reference.batch_id,
            reference.sequence,
            reference.rollback,
        )?;
        let op = active
            .items
            .iter()
            .find(|op| &op.path == path)
            .ok_or(BrokerError::CorruptWriterState)?;
        return Ok(!active.rollback && op.previous_revision.is_none());
    }
    Ok(false)
}

impl FsVaultBroker {
    /// 最新优先列出这条路径至多 `limit` 条可见历史，并标出哪些是创建。
    ///
    /// 只读；和 [`FsVaultBroker::list_history`] 一样，写入在途或读的过程中目录变了就失败关闭。
    pub fn history_lineage(
        &self,
        path: &VaultPath,
        limit: usize,
    ) -> Result<HistoryLineage, BrokerError> {
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = query::readable(self)?;
        let catalog = catalog::load(self, path)?;
        let digest = query::checksum(&catalog)?;
        let mut items = Vec::new();
        for item in &catalog.entries {
            if item.sequence <= state.committed_through && super::batch::visible(self, item)? {
                items.push(item);
            }
        }
        items.sort_by(|a, b| query::newest_first(a, b));
        let more = items.len() > limit;
        let mut entries = Vec::with_capacity(items.len().min(limit));
        for item in items.into_iter().take(limit) {
            entries.push(HistoryLineageEntry {
                entry: item.entry.clone(),
                created: created(self, path, item)?,
            });
        }
        query::unchanged(self, &state)?;
        if query::checksum(&catalog::load(self, path)?)? != digest {
            return Err(BrokerError::HistoryChanged);
        }
        Ok(HistoryLineage { entries, more })
    }
}
