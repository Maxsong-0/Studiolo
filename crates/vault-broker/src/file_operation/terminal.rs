//! typed 终态共享旧键目录；临时文件留在新固定 schema 内，旧读者显式识别跨类型冲突。
use super::{atomic, disk, record::Terminal};
use crate::{
    BrokerError, FsVaultBroker,
    mutation::replay::{self, Replay},
    writer::{directory, disk as codec},
};
use std::path::Path;

pub(crate) fn load(
    broker: &FsVaultBroker,
    frozen: &super::frozen::Frozen,
) -> Result<Option<Terminal>, BrokerError> {
    match replay::load(&broker.data, &frozen.key_hash, frozen.pair_id)? {
        Some(Replay::FileOperation(value)) if value.frozen.matches(frozen) => Ok(Some(*value)),
        Some(_) => Err(BrokerError::IdempotencyConflict),
        None => Ok(None),
    }
}
pub(crate) fn save(broker: &FsVaultBroker, value: &Terminal) -> Result<(), BrokerError> {
    value.validate(broker.writer.binding.pair_id)?;
    if let Some(existing) = load(broker, &value.frozen)? {
        return if existing == *value {
            Ok(())
        } else {
            Err(BrokerError::CorruptWriterState)
        };
    }
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    let (_, count, total) = replay::scan(&broker.data, Some(value.frozen.pair_id))?;
    if count >= codec::REPLAY_COUNT
        || bytes.len() > codec::RECORD_LIMIT
        || total
            .checked_add(bytes.len())
            .is_none_or(|v| v > codec::REPLAY_BYTES)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let staging = disk::parent(broker, disk::Folder::Records)?;
    disk::reserve(broker, disk::Folder::Records, 0, bytes.len())?;
    let parent = directory(&broker.data, "idempotency")?;
    let name = format!("{}.json", value.frozen.key_hash);
    atomic::write(broker, &staging, &name, &bytes, &parent, true, || {
        broker.writer.check(&broker.root, &broker.data)?;
        if replay::scan(&broker.data, Some(value.frozen.pair_id))? != (false, count, total)
            || codec::read(&parent, &name, codec::RECORD_LIMIT)?.is_some()
        {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    })?;
    FsVaultBroker::sync_directory(&parent, Path::new("."))?;
    Ok(())
}
