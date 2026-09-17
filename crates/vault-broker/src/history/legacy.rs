//! 旧 L0 只从成功回执找到路径绑定；没有原始作者与时间时保持 unknown。

use super::{HistoryEntry, catalog::Item};
use crate::writer::{directory, disk};
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    mutation::record::Outcome,
    mutation::replay::{self, Replay},
    sha256,
};
use std::collections::BTreeMap;

pub(super) fn load(broker: &FsVaultBroker, path: &VaultPath) -> Result<Vec<Item>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = directory(&broker.data, "idempotency")?;
    let history = directory(&broker.data, "history")?;
    let mut found = BTreeMap::new();
    let mut total = 0usize;
    for (index, item) in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate legacy history", e))?
        .enumerate()
    {
        if index >= disk::REPLAY_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let item = item.map_err(|e| BrokerError::io("inspect legacy history", e))?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix(".json").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        let bytes =
            disk::read(&dir, &name, disk::RECORD_LIMIT)?.ok_or(BrokerError::CorruptWriterState)?;
        total = total
            .checked_add(bytes.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > disk::REPLAY_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let (receipt, recorded) = match replay::decode(&bytes, Some(broker.writer.binding.pair_id))?
        {
            Replay::FileOperation(_) => continue,
            Replay::Legacy(value) => (value.receipt, false),
            Replay::Current(value) => (
                match value.outcome {
                    Outcome::Committed { receipt } => Some(receipt),
                    _ => None,
                },
                value.recording.is_some(),
            ),
        };
        let Some(receipt) = receipt.filter(|r| &r.path == path) else {
            continue;
        };
        // 新格式已经承诺持久索引，索引缺失不能回退成旧版本的部分列表。
        if recorded {
            return Err(BrokerError::CorruptWriterState);
        }
        for revision in receipt
            .previous_revision
            .into_iter()
            .chain([receipt.revision])
        {
            let key = revision.to_string();
            let logical = VaultPath::parse(&key).map_err(|_| BrokerError::CorruptWriterState)?;
            let file = match FsVaultBroker::open_rooted_file(&history, &logical) {
                Ok(file) => file,
                Err(BrokerError::NotFound(_)) => continue,
                Err(error) => return Err(error),
            };
            let metadata = file
                .metadata()
                .map_err(|e| BrokerError::io("stat legacy history", e))?;
            FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
            if metadata.len() > disk::CONTENT_LIMIT as u64 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            found.entry(key.clone()).or_insert(Item {
                sequence: 0,
                recording: None,
                file_operation_id: None,
                entry: HistoryEntry {
                    id: sha256(format!("legacy:{}:{key}", path.as_str()).as_bytes()),
                    revision,
                    bytes: metadata.len(),
                    authored_at_ms: None,
                    author: None,
                    archived_at_ms: None,
                    archived_by: None,
                },
            });
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(found.into_values().collect())
}
