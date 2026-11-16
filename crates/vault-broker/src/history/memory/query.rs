use super::*;
use crate::{ChangePage, HistoryContent, HistoryPage, MemoryVaultBroker};
impl MemoryVaultBroker {
    /// 内存替身的单页历史，游标仍绑定路径和当前目录摘要。
    pub fn list_history(
        &self,
        path: &VaultPath,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<HistoryPage, BrokerError> {
        if path.is_protected() {
            return Err(BrokerError::ProtectedPath(path.clone()));
        }
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let items = self
            .change_history
            .entries
            .get(path)
            .cloned()
            .unwrap_or_default();
        let digest = crate::sha256(
            &serde_json::to_vec(&items).map_err(|_| BrokerError::CorruptWriterState)?,
        );
        let path_hash = crate::sha256(path.as_str().as_bytes());
        let offset = cursor
            .map(|v| super::super::query::parse_cursor(v, &path_hash, &digest))
            .transpose()?
            .unwrap_or(0);
        if offset > items.len() {
            return Err(BrokerError::InvalidHistoryCursor);
        }
        let end = (offset + limit).min(items.len());
        Ok(HistoryPage {
            entries: items[offset..end].to_vec(),
            next_cursor: (end < items.len()).then(|| format!("h1:{path_hash}:{digest}:{end}")),
        })
    }
    /// 只读取该路径仍可查询的历史原字节。
    pub fn read_history(
        &self,
        path: &VaultPath,
        revision: Revision,
        max_bytes: u64,
    ) -> Result<HistoryContent, BrokerError> {
        if path.is_protected() {
            return Err(BrokerError::ProtectedPath(path.clone()));
        }
        if !self
            .change_history
            .entries
            .get(path)
            .is_some_and(|v| v.iter().any(|i| i.revision == revision))
        {
            return Err(BrokerError::NotFound(path.clone()));
        }
        let bytes = self
            .history
            .get(&revision)
            .ok_or(BrokerError::CorruptWriterState)?;
        if bytes.len() as u64 > max_bytes {
            return Err(BrokerError::ReadLimitExceeded {
                path: path.clone(),
                max_bytes,
            });
        }
        if Revision::compute(self.vault_uuid, path, bytes) != revision {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(HistoryContent {
            vault_uuid: self.vault_uuid,
            path: path.clone(),
            revision,
            content: bytes.clone(),
        })
    }
    /// 和磁盘替身相同的两桶窗口及固定提交上限；无人工缺失桶。
    pub fn list_changes(
        &self,
        limit: usize,
        before: Option<&str>,
    ) -> Result<ChangePage, BrokerError> {
        use super::super::feed::{query, storage};
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let (through, mut bucket, mut offset) = before
            .map(|v| query::parse(v, self.change_history.sequence))
            .transpose()?
            .unwrap_or((
                self.change_history.sequence,
                storage::number(self.change_history.sequence),
                0,
            ));
        let mut page = ChangePage {
            entries: Vec::new(),
            next_cursor: None,
            through_sequence: through,
            history_incomplete: false,
            retired: false,
        };
        if through == 0 {
            return Ok(page);
        }
        for _ in 0..2 {
            let mut rows: Vec<_> = self
                .change_history
                .changes
                .iter()
                .filter(|r| r.sequence <= through && storage::number(r.sequence) == bucket)
                .collect();
            rows.sort_by(|a, b| {
                b.sequence
                    .cmp(&a.sequence)
                    .then_with(|| a.path.cmp(&b.path))
            });
            if offset > rows.len() {
                return Err(BrokerError::HistoryChanged);
            }
            let end = (offset + limit - page.entries.len()).min(rows.len());
            page.entries
                .extend(rows[offset..end].iter().map(|r| (*r).clone()));
            offset = end;
            if offset < rows.len() {
                page.next_cursor = Some(query::cursor(through, bucket, offset));
                break;
            }
            if bucket == 0 {
                page.next_cursor = None;
                break;
            }
            bucket -= 1;
            offset = 0;
            page.next_cursor = Some(query::cursor(through, bucket, offset));
            if page.entries.len() == limit {
                break;
            }
        }
        Ok(page)
    }
}
