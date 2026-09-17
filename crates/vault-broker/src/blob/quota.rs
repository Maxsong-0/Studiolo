//! 所有自有 stage（包括孤儿和接收残留）按逻辑字节计费；未知对象整体拒绝。
use super::{OBJECT_LIMIT, STAGE_BYTES, STAGE_COUNT};
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    writer::{directory, disk},
};
use cap_std::fs::{Dir, MetadataExt as _};

pub(crate) fn reserve(broker: &FsVaultBroker, incoming: u64) -> Result<(), BrokerError> {
    if incoming > OBJECT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    let stages = directory(&broker.root, ".workbench/staging")?;
    let mut total = incoming;
    for (count, entry) in stages
        .entries()
        .map_err(|e| BrokerError::io("enumerate blob stages", e))?
        .enumerate()
    {
        if count >= STAGE_COUNT - 1 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect blob stage", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix(".stage").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        let logical = VaultPath::parse(&name).map_err(|_| BrokerError::CorruptWriterState)?;
        let metadata = stages
            .symlink_metadata(&name)
            .map_err(|e| BrokerError::io("stat blob stage", e))?;
        FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
        if metadata.nlink() != 1 || metadata.len() > OBJECT_LIMIT {
            return Err(BrokerError::CorruptWriterState);
        }
        total = total
            .checked_add(metadata.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > STAGE_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    broker.writer.check(&broker.root, &broker.data)
}

pub(crate) fn reserve_preservation(root: &Dir, incoming: u64) -> Result<(), BrokerError> {
    let mut count = 0;
    let mut bytes = incoming;
    for (index, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("enumerate preserved blobs", e))?
        .enumerate()
    {
        if index >= 50_000 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect preserved blob", e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| n.starts_with(".studiolo-blob-")) else {
            continue;
        };
        count += 1;
        let value = root
            .symlink_metadata(name)
            .map_err(|e| BrokerError::io("stat blob temporary", e))?;
        FsVaultBroker::validate_file_metadata(&crate::blob::store::logical(name)?, &value)?;
        if count >= 64 || value.len() > OBJECT_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        bytes = bytes
            .checked_add(value.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if bytes > STAGE_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    Ok(())
}
