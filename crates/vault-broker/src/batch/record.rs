//! journal 只记录私有 stage、history 指针与 revision；不放正文。
use super::BatchReceipt;
use crate::mutation::record::{Operation, Phase};
use crate::{BrokerError, FsVaultBroker, VaultUuid, writer::disk};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    pub actor: crate::MutationActor,
    pub requested_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Active {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Provenance>,
    pub batch_id: String,
    pub request_hash: String,
    pub preview_hash: String,
    pub fingerprint: String,
    pub sequence: u64,
    pub rollback: bool,
    pub phase: Phase,
    pub items: Vec<Operation>,
}
impl Active {
    pub(crate) fn validate(&self) -> Result<(), BrokerError> {
        if self.batch_id.is_empty()
            || self.batch_id.len() > 256
            || self.sequence == 0
            || self.items.is_empty()
            || self.items.len() > 32
            || !disk::hex(&self.request_hash)
            || !disk::hex(&self.preview_hash)
            || !disk::hex(&self.fingerprint)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if let Some(metadata) = &self.metadata {
            crate::MutationMetadata::new(metadata.actor.clone(), crate::MutationKind::Replace)
                .map_err(|_| BrokerError::CorruptWriterState)?;
            if metadata.requested_at_ms.is_some_and(|v| v < 0) {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        for item in &self.items {
            item.validate(false)?;
            if item.recording.is_some()
                || item.sequence != self.sequence
                || item.phase != Phase::Prepared
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        if self.items.windows(2).any(|p| p[0].path >= p[1].path) {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
    pub(crate) fn receipt(&self) -> BatchReceipt {
        BatchReceipt {
            batch_id: self.batch_id.clone(),
            fingerprint: self.fingerprint.clone(),
            mutations: self.items.iter().map(Operation::receipt).collect(),
            rolled_back: self.rollback,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Terminal {
    pub pair: VaultUuid,
    pub active: Active,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed: Option<Active>,
}
const NAME_PREFIX: &str = "batch-";
const NAME_SUFFIX: &str = ".json";
/// 终态记录的文件名：写者数据根这一层的 `batch-<批次号的 SHA-256>.json`。导出扫描也按它核（`trusted_export`）。
pub(crate) fn name(id: &str) -> String {
    format!("{NAME_PREFIX}{}{NAME_SUFFIX}", crate::sha256(id.as_bytes()))
}
/// 写者数据根这一层名字像终态记录的文件；读它时还要核名字等于 `name(批次号)`。
pub(crate) fn looks_like_terminal(name: &str) -> bool {
    name.starts_with(NAME_PREFIX) && name.ends_with(NAME_SUFFIX)
}
pub(super) fn terminal(broker: &FsVaultBroker, id: &str) -> Result<Option<Active>, BrokerError> {
    Ok(load_terminal(broker, id)?.map(|t| t.active))
}
fn load_terminal(broker: &FsVaultBroker, id: &str) -> Result<Option<Terminal>, BrokerError> {
    disk::read(&broker.data, &name(id), disk::RECORD_LIMIT)?
        .map(|bytes| {
            let record: Terminal = disk::decode(&bytes)?;
            record.active.validate()?;
            if record.pair != broker.writer.binding.pair_id
                || record.active.batch_id != id
                || record.active.phase != Phase::Completed
            {
                return Err(BrokerError::CorruptWriterState);
            }
            if let Some(prior) = &record.committed {
                prior.validate()?;
                if !record.active.rollback
                    || prior.rollback
                    || prior.phase != Phase::Completed
                    || prior.batch_id != id
                    || prior.sequence > record.active.sequence
                    || prior.request_hash != record.active.request_hash
                    || prior.preview_hash != record.active.preview_hash
                {
                    return Err(BrokerError::CorruptWriterState);
                }
            }
            Ok(record)
        })
        .transpose()
}
pub(super) fn save_terminal(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    let prior = load_terminal(broker, &active.batch_id)?;
    let committed = if active.rollback {
        prior.and_then(|t| {
            if t.active.rollback {
                t.committed
            } else {
                Some(t.active)
            }
        })
    } else {
        None
    };
    disk::save(
        &broker.data,
        &name(&active.batch_id),
        &Terminal {
            pair: broker.writer.binding.pair_id,
            active: active.clone(),
            committed,
        },
        || broker.writer.check(&broker.root, &broker.data),
    )
}
/// 历史条目以序号与方向绑定固定终态；原提交在回滚后仍保留。
pub(crate) fn history_source(
    broker: &FsVaultBroker,
    id: &str,
    sequence: u64,
    rollback: bool,
) -> Result<(Active, bool), BrokerError> {
    let terminal = load_terminal(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
    let visible = !(terminal.active.rollback && !rollback && terminal.active.sequence == sequence);
    for active in std::iter::once(&terminal.active).chain(terminal.committed.iter()) {
        if active.sequence == sequence && active.rollback == rollback {
            return Ok((active.clone(), visible));
        }
    }
    Err(BrokerError::CorruptWriterState)
}

/// 完成批次仍引用旧 revision，历史整理必须保留其回滚证据。
pub(crate) fn protected(
    broker: &FsVaultBroker,
    path: &crate::VaultPath,
) -> Result<std::collections::BTreeSet<crate::Revision>, BrokerError> {
    let mut revisions = std::collections::BTreeSet::new();
    let mut count = 0;
    for entry in broker
        .data
        .entries()
        .map_err(|e| BrokerError::io("enumerate batch receipts", e))?
    {
        let entry = entry.map_err(|e| BrokerError::io("inspect batch receipt", e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| looks_like_terminal(n)) else {
            continue;
        };
        count += 1;
        if count > 4096 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let bytes = disk::read(&broker.data, name, disk::RECORD_LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?;
        let record: Terminal = disk::decode(&bytes)?;
        let active =
            terminal(broker, &record.active.batch_id)?.ok_or(BrokerError::CorruptWriterState)?;
        if name != self::name(&active.batch_id) {
            return Err(BrokerError::CorruptWriterState);
        }
        for item in active.items.iter().filter(|item| &item.path == path) {
            revisions.extend(item.previous_revision);
            revisions.insert(item.target_revision);
        }
    }
    Ok(revisions)
}
