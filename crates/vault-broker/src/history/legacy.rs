//! 旧 L0 只从成功回执找到路径绑定；没有原始作者与时间时保持 unknown。
//!
//! 某路径还没有历史目录时要看旧回执里有没有它。原来每次都把全部重放散件读一遍；现在开库之后第一次用到时读一遍、
//! 建成索引（T-249 G5），之后只在写成新的终态时添一条。旧格式（没有描述符）的回执早就不再产生；带描述符的已提交路径
//! 另记一张表，用来核「已经承诺持久索引、却没有目录」。索引只记散件里的：打包删掉、还原写回重放散件时整份作废，
//! 下次用到重建（打了包的路径在段的已提交表里另查）。

use super::{HistoryEntry, catalog::Item};
use crate::writer::{directory, disk};
use crate::{
    BrokerError, FsVaultBroker, MutationReceipt, VaultPath,
    mutation::record::{Outcome, Terminal},
    mutation::replay::{self, Replay},
    records::WriterRecords,
    sha256,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::PoisonError,
};

/// 重放散件里和历史有关的那部分。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LegacyIndex {
    /// 不带描述符的已提交回执，按路径。
    receipts: HashMap<VaultPath, Vec<MutationReceipt>>,
    /// 带描述符的已提交路径：它们一定有历史目录。
    recorded: HashSet<VaultPath>,
}
impl LegacyIndex {
    fn add(&mut self, receipt: MutationReceipt, recorded: bool) {
        if recorded {
            self.recorded.insert(receipt.path);
        } else {
            self.receipts
                .entry(receipt.path.clone())
                .or_default()
                .push(receipt);
        }
    }
}

/// 把重放散件读一遍建索引；条数、字节的上限和原来每次全读时一样。
fn build(broker: &FsVaultBroker) -> Result<LegacyIndex, BrokerError> {
    let dir = directory(&broker.data, "idempotency")?;
    let mut index = LegacyIndex::default();
    let mut total = 0usize;
    for (count, item) in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate legacy history", e))?
        .enumerate()
    {
        #[cfg(test)]
        disk::probe::listed();
        if count >= disk::REPLAY_COUNT {
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
        if let Some(receipt) = receipt {
            index.add(receipt, recorded);
        }
    }
    Ok(index)
}

/// 拿着索引做事；还没建就先建。
fn with_index<T>(
    broker: &FsVaultBroker,
    use_it: impl FnOnce(&LegacyIndex) -> T,
) -> Result<T, BrokerError> {
    let mut cached = broker
        .legacy_history
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if cached.is_none() {
        *cached = Some(build(broker)?);
    }
    let index = cached.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    // 用例构建：每次都和盘上重建的比（不算进读取计数），哪一处没添上就在这里红。
    // 盘上建不成（用例故意弄坏了）不比：生产里这一步也看不见。
    #[cfg(test)]
    if let Ok(fresh) = disk::probe::uncounted(|| build(broker)) {
        assert_eq!(index, &fresh, "旧格式历史索引和盘上不一致");
    }
    Ok(use_it(index))
}

/// 一条正文终态写成了：建好的索引添上它。
pub(crate) fn saved(broker: &FsVaultBroker, terminal: &Terminal) {
    let Outcome::Committed { receipt } = &terminal.outcome else {
        return;
    };
    if let Some(index) = broker
        .legacy_history
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
    {
        index.add(receipt.clone(), terminal.recording.is_some());
    }
}

/// 记不准了（写没确认、还原过）：下次用到时重建。
pub(crate) fn forget(broker: &FsVaultBroker) {
    *broker
        .legacy_history
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
}

pub(super) fn load(broker: &FsVaultBroker, path: &VaultPath) -> Result<Vec<Item>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    // 打包的都是新格式：已经承诺持久索引，这条路径却没有目录，就是坏的。
    if broker.archive()?.committed(path) {
        return Err(BrokerError::CorruptWriterState);
    }
    let (recorded, receipts) = with_index(broker, |index| {
        (
            index.recorded.contains(path),
            index.receipts.get(path).cloned().unwrap_or_default(),
        )
    })?;
    // 新格式已经承诺持久索引，索引缺失不能回退成旧版本的部分列表。
    if recorded {
        return Err(BrokerError::CorruptWriterState);
    }
    let history = directory(&broker.data, "history")?;
    let mut found = BTreeMap::new();
    for receipt in receipts {
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
                batch: None,
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
