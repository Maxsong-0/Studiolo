//! 逆向核对永久终态的母本引用；整个请求目录被删掉也不能伪装成「没有审计历史」。
use crate::{
    BrokerError, FsVaultBroker,
    mutation::replay::{self, Replay},
    recorded,
    writer::{directory, disk},
};
use std::time::Instant;
pub(super) fn verify(broker: &FsVaultBroker, deadline: Instant) -> Result<(), BrokerError> {
    let directory = directory(&broker.data, "idempotency")?;
    let mut total = 0usize;
    for (index, entry) in directory
        .entries()
        .map_err(|e| BrokerError::io("枚举封存终态", e))?
        .enumerate()
    {
        super::check_deadline(deadline)?;
        if index >= disk::REPLAY_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let name = entry
            .map_err(|e| BrokerError::io("读取封存终态名", e))?
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        let key = name
            .strip_suffix(".json")
            .filter(|key| disk::hex(key))
            .ok_or(BrokerError::CorruptWriterState)?;
        let bytes = disk::read(&directory, &name, disk::RECORD_LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?;
        total = total
            .checked_add(bytes.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > disk::REPLAY_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        match replay::decode(&bytes, Some(broker.writer.binding.pair_id))? {
            Replay::Legacy(value) if value.receipt.is_none() => {
                return Err(BrokerError::MutationUnresolved);
            }
            Replay::Legacy(_) => {}
            Replay::Current(value) => {
                recorded::validate_terminal(broker, &value)?;
                if let Some(reference) = &value.recording
                    && recorded::store::load(broker, reference)?.key_hash != key
                {
                    return Err(BrokerError::CorruptWriterState);
                }
            }
            Replay::FileOperation(value) => {
                let request =
                    crate::file_operation::request::load(broker, &value.frozen.operation_id)?
                        .ok_or(BrokerError::CorruptWriterState)?;
                if value.frozen.key_hash != key || value.frozen != request.frozen {
                    return Err(BrokerError::CorruptWriterState);
                }
            }
        }
    }
    super::check_deadline(deadline)
}
