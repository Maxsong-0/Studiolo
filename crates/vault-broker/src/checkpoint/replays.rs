//! 逆向核对永久终态的母本引用；整个请求目录被删掉也不能伪装成「没有审计历史」。
use crate::{
    BrokerError,
    archive::Part,
    mutation::replay::{self, Replay},
    recorded,
    records::WriterRecords,
    writer::{directory, disk},
};
use std::time::Instant;
pub(super) fn verify(records: &dyn WriterRecords, deadline: Instant) -> Result<(), BrokerError> {
    let archive = records.archive()?;
    let directory = directory(records.data(), "idempotency")?;
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
        // 打包提交了、散件还没删完的那一刻：两份必须逐字节相同，段里那份在下面核。
        if let Some(packed) = archive.read(records.data(), Part::Replay, key)? {
            if packed != bytes {
                return Err(BrokerError::CorruptWriterState);
            }
            continue;
        }
        check(records, key, &bytes)?;
    }
    for pack in archive.packs() {
        super::check_deadline(deadline)?;
        let bytes = archive
            .read(records.data(), Part::Replay, &pack.key)?
            .ok_or(BrokerError::CorruptWriterState)?;
        check(records, &pack.key, &bytes)?;
    }
    super::check_deadline(deadline)
}

/// 一条终态和它的母本对得上；散件和打了包的同一个判法。
fn check(records: &dyn WriterRecords, key: &str, bytes: &[u8]) -> Result<(), BrokerError> {
    match replay::decode(bytes, Some(records.pair_id()))? {
        Replay::Legacy(value) if value.receipt.is_none() => {
            return Err(BrokerError::MutationUnresolved);
        }
        Replay::Legacy(_) => {}
        Replay::Current(value) => {
            recorded::validate_terminal(records, &value)?;
            if let Some(reference) = &value.recording
                && recorded::store::load(records, reference)?.key_hash != key
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Replay::FileOperation(value) => {
            let request =
                crate::file_operation::request::load(records, &value.frozen.operation_id)?
                    .ok_or(BrokerError::CorruptWriterState)?;
            if value.frozen.key_hash != key || value.frozen != request.frozen {
                return Err(BrokerError::CorruptWriterState);
            }
        }
    }
    Ok(())
}
