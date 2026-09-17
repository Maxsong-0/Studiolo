//! 不扫无限请求目录：条数、单文件和总字节都有上限，并为两份 ACK 留出空间。

use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};

pub(crate) fn reserve(broker: &FsVaultBroker, incoming: usize) -> Result<(), BrokerError> {
    let Some(dir) = crate::history::store::open(broker, false)? else {
        return Ok(());
    };
    let requests = directory(&dir, "requests")?;
    let mut records = 0usize;
    let mut bytes = incoming;
    for (index, entry) in requests
        .entries()
        .map_err(|e| BrokerError::io("enumerate content metadata", e))?
        .enumerate()
    {
        if index >= 3 * disk::REPLAY_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect content metadata", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        let (prefix, descriptor) = if let Some(id) = name
            .strip_suffix("-intent.json.ack")
            .or_else(|| name.strip_suffix("-outcome.json.ack"))
        {
            (id, false)
        } else {
            (
                name.strip_suffix(".json")
                    .ok_or(BrokerError::CorruptWriterState)?,
                true,
            )
        };
        if !disk::hex(prefix) {
            return Err(BrokerError::CorruptWriterState);
        }
        let logical =
            crate::VaultPath::parse(&name).map_err(|_| BrokerError::CorruptWriterState)?;
        let metadata = entry
            .metadata()
            .map_err(|e| BrokerError::io("stat content metadata", e))?;
        FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
        if metadata.len() > disk::RECORD_LIMIT as u64 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        if descriptor {
            records += 1;
            bytes = bytes
                .checked_add(metadata.len() as usize)
                .ok_or(BrokerError::WriterLimitExceeded)?;
        }
    }
    if records >= disk::REPLAY_COUNT
        || bytes.saturating_add((records + 1) * 2048) > 32 * 1024 * 1024
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    crate::history::store::open(broker, false)?;
    Ok(())
}
