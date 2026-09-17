//! 先完整检查，再原子退役索引、删除对象；中断留下可继续整理的显式垃圾清单。

use super::{HistoryPruneReport, HistoryRetention, catalog, query, store};
use crate::writer::directory;
use crate::{BrokerError, FsVaultBroker, VaultPath, mutation};
use std::{collections::BTreeSet, path::Path};

impl FsVaultBroker {
    /// 同时超出最近版本数量及天数才整理；未知时间保留。owner 还须排除检查点冻结窗口。
    pub fn prune_history(
        &mut self,
        path: &VaultPath,
        retention: HistoryRetention,
        max_entries: usize,
    ) -> Result<HistoryPruneReport, BrokerError> {
        crate::maintenance::ensure_idle(self)?;
        if retention.versions < 50 || retention.days < 30 || !(1..=64).contains(&max_entries) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = query::readable(self)?;
        if state.operation.is_some() || self.content_audit_status()?.restore_unconfirmed {
            return Err(BrokerError::MutationUnresolved);
        }
        let mut catalog = catalog::load(self, path)?;
        let before = query::checksum(&catalog)?;
        let now = store::now().ok_or(BrokerError::WriterLimitExceeded)?;
        let age = i64::from(retention.days)
            .checked_mul(86_400_000)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let cutoff = now
            .checked_sub(age)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let current = crate::blob::current::observe(self, path)?.revision();
        let protected = crate::file_operation::tombstone::protected(self, path)?;
        if catalog
            .garbage
            .iter()
            .any(|revision| protected.contains(revision))
        {
            return Err(BrokerError::MutationUnresolved);
        }
        let mut removed = 0;
        if catalog.garbage.is_empty() {
            let mut ordered = catalog.entries.clone();
            ordered.sort_by(|a, b| {
                b.sequence
                    .cmp(&a.sequence)
                    .then_with(|| b.entry.authored_at_ms.cmp(&a.entry.authored_at_ms))
                    .then_with(|| a.entry.id.cmp(&b.entry.id))
            });
            let retire: BTreeSet<_> = ordered
                .iter()
                .skip(retention.versions)
                .filter(|item| {
                    item.entry.archived_at_ms.is_some_and(|at| at < cutoff)
                        && Some(item.entry.revision) != current
                        && !protected.contains(&item.entry.revision)
                })
                .take(max_entries)
                .map(|item| item.entry.id.clone())
                .collect();
            removed = retire.len();
            let candidates: BTreeSet<_> = catalog
                .entries
                .iter()
                .filter(|item| retire.contains(&item.entry.id))
                .map(|item| item.entry.revision)
                .collect();
            catalog
                .entries
                .retain(|item| !retire.contains(&item.entry.id));
            for revision in candidates {
                if !catalog
                    .entries
                    .iter()
                    .any(|item| item.entry.revision == revision)
                {
                    catalog.garbage.push(revision);
                }
            }
        }
        if catalog.garbage.len() > max_entries {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let history = directory(&self.data, "history")?;
        let mut pins = Vec::new();
        let mut total = 0usize;
        for revision in &catalog.garbage {
            let name = revision.to_string();
            if crate::blob::store::exists(&history, &name)? {
                let proof = super::blob::object(self, path, *revision)?;
                total = total
                    .checked_add(proof.identity.bytes as usize)
                    .ok_or(BrokerError::WriterLimitExceeded)?;
                if total > crate::blob::STAGE_BYTES as usize {
                    return Err(BrokerError::WriterLimitExceeded);
                }
                pins.push((name, proof.node));
            }
        }
        query::unchanged(self, &state)?;
        if query::checksum(&catalog::load(self, path)?)? != before {
            return Err(BrokerError::HistoryChanged);
        }
        // 先退役：删除中断不会留下声称可恢复、实际已缺字节的可见条目。
        catalog::save(self, &mut catalog)?;
        mutation::fault(self, 15)?;
        for (name, pin) in &pins {
            self.writer.check(&self.root, &self.data)?;
            crate::blob::file::check_file(&history, &crate::blob::store::logical(name)?, pin)?;
            history
                .remove_file(name)
                .map_err(|e| BrokerError::io("prune retired history", e))?;
            FsVaultBroker::sync_directory(&history, Path::new("."))?;
            mutation::fault(self, 16)?;
        }
        self.writer.check(&self.root, &self.data)?;
        catalog.garbage.clear();
        catalog::save(self, &mut catalog)?;
        Ok(HistoryPruneReport {
            removed_entries: removed,
            removed_objects: pins.len(),
            retained_entries: catalog.entries.len(),
        })
    }
}
