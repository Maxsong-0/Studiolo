//! typed 终态共享旧键目录；临时文件留在新固定 schema 内，旧读者显式识别跨类型冲突。
use super::{atomic, disk, record::Terminal};
use crate::{
    BrokerError, FsVaultBroker,
    loose_usage::LooseFolder,
    mutation::replay::{self, Replay},
    records::WriterRecords,
    writer::{directory, disk as codec},
};
use std::path::Path;

pub(crate) fn load(
    source: &dyn WriterRecords,
    frozen: &super::frozen::Frozen,
) -> Result<Option<Terminal>, BrokerError> {
    match replay::load(source, &frozen.key_hash, frozen.pair_id)? {
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
    // 读内存里的散件用量，不全目录扫（T-249 G5）；动手之前已经按同一把尺留过位置。
    let replays = broker.loose_usage()?.tally(LooseFolder::Replays);
    if replays.records >= codec::REPLAY_COUNT
        || bytes.len() > codec::RECORD_LIMIT
        || (replays.bytes as usize)
            .checked_add(bytes.len())
            .is_none_or(|v| v > codec::REPLAY_BYTES)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let staging = disk::parent(broker, disk::Folder::Records)?;
    disk::reserve(broker, disk::Folder::Records, 0, bytes.len())?;
    let parent = directory(&broker.data, "idempotency")?;
    let name = format!("{}.json", value.frozen.key_hash);
    let written = atomic::write(broker, &staging, &name, &bytes, &parent, true, || {
        broker.writer.check(&broker.root, &broker.data)?;
        if codec::read(&parent, &name, codec::RECORD_LIMIT)?.is_some() {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(())
    })
    .and_then(|()| FsVaultBroker::sync_directory(&parent, Path::new(".")));
    broker.loose_touch(LooseFolder::Replays, &name);
    written
}
