//! journal 只记录私有 stage、history 指针与 revision；不放正文。
use super::BatchReceipt;
use crate::mutation::record::{Operation, Phase};
use crate::{BrokerError, FsVaultBroker, VaultUuid, writer::disk};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Active {
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
}
fn name(id: &str) -> String {
    format!("batch-{}.json", crate::sha256(id.as_bytes()))
}
pub(super) fn terminal(broker: &FsVaultBroker, id: &str) -> Result<Option<Active>, BrokerError> {
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
            Ok(record.active)
        })
        .transpose()
}
pub(super) fn save_terminal(broker: &FsVaultBroker, active: &Active) -> Result<(), BrokerError> {
    disk::save(
        &broker.data,
        &name(&active.batch_id),
        &Terminal {
            pair: broker.writer.binding.pair_id,
            active: active.clone(),
        },
        || broker.writer.check(&broker.root, &broker.data),
    )
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
        let Some(name) = name
            .to_str()
            .filter(|n| n.starts_with("batch-") && n.ends_with(".json"))
        else {
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
        if name != format!("batch-{}.json", crate::sha256(active.batch_id.as_bytes())) {
            return Err(BrokerError::CorruptWriterState);
        }
        for item in active.items.iter().filter(|item| &item.path == path) {
            revisions.extend(item.previous_revision);
            revisions.insert(item.target_revision);
        }
    }
    Ok(revisions)
}
