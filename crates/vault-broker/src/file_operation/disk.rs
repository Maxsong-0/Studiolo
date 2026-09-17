//! 元数据先验总量固定且逐项 nofollow；每个可变记录还要匹配调用前的节点与摘要。
use super::{atomic, limits, store};
use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};
use cap_std::fs::{Dir, MetadataExt as _};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) enum Folder {
    Records,
    Tombstones,
    Acks,
    Evidence,
}
impl Folder {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Records => "records",
            Self::Tombstones => "tombstones",
            Self::Acks => "acks",
            Self::Evidence => "evidence",
        }
    }
    pub(crate) fn budget(self) -> (usize, usize) {
        match self {
            Self::Records => (limits::RECORD_COUNT, limits::RECORD_BYTES),
            Self::Tombstones => (limits::TOMBSTONE_COUNT, limits::TOMBSTONE_BYTES),
            Self::Acks => (limits::RECORD_COUNT * 2, limits::TOMBSTONE_BYTES),
            Self::Evidence => (limits::RECORD_COUNT, limits::TOMBSTONE_BYTES),
        }
    }
    fn validate_name(self, name: &str) -> Result<(), BrokerError> {
        let base = name
            .strip_suffix(".json")
            .ok_or(BrokerError::CorruptWriterState)?;
        let id = match self {
            Self::Acks => base
                .strip_suffix("-intent")
                .or_else(|| base.strip_suffix("-outcome"))
                .ok_or(BrokerError::CorruptWriterState)?,
            _ => base,
        };
        if !disk::hex(id) {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
}
pub(crate) fn parent(broker: &FsVaultBroker, folder: Folder) -> Result<Dir, BrokerError> {
    directory(&store::required(broker)?.records, folder.name())
}

fn scan(broker: &FsVaultBroker, folder: Folder) -> Result<(Vec<String>, usize), BrokerError> {
    let root = parent(broker, folder)?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    let mut temporaries = 0usize;
    let (count, byte_limit) = folder.budget();
    for (index, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("enumerate file operation metadata", e))?
        .enumerate()
    {
        if index >= count + atomic::TEMP_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect file operation metadata", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        let temporary = atomic::temporary(&name)?;
        if temporary {
            temporaries += 1;
            if temporaries > atomic::TEMP_LIMIT {
                return Err(BrokerError::WriterLimitExceeded);
            }
        } else {
            folder.validate_name(&name)?;
            if result.len() >= count {
                return Err(BrokerError::WriterLimitExceeded);
            }
        }
        let metadata = root
            .symlink_metadata(&name)
            .map_err(|e| BrokerError::io("stat file operation metadata", e))?;
        FsVaultBroker::validate_file_metadata(&crate::blob::store::logical(&name)?, &metadata)?;
        if metadata.nlink() != 1 || metadata.len() > limits::REQUEST_LIMIT as u64 {
            return Err(BrokerError::CorruptWriterState);
        }
        bytes = bytes
            .checked_add(metadata.len() as usize)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if bytes > byte_limit {
            return Err(BrokerError::WriterLimitExceeded);
        }
        if !temporary {
            result.push(name);
        }
    }
    result.sort();
    store::required(broker)?;
    Ok((result, bytes))
}

pub(crate) fn names(broker: &FsVaultBroker, folder: Folder) -> Result<Vec<String>, BrokerError> {
    scan(broker, folder).map(|value| value.0)
}

pub(crate) fn usage(broker: &FsVaultBroker, folder: Folder) -> Result<usize, BrokerError> {
    scan(broker, folder).map(|v| v.1)
}

pub(crate) fn reserve(
    broker: &FsVaultBroker,
    folder: Folder,
    count: usize,
    bytes: usize,
) -> Result<(), BrokerError> {
    let (entries, total) = scan(broker, folder)?;
    let (max_count, max_bytes) = folder.budget();
    if entries
        .len()
        .checked_add(count)
        .is_none_or(|v| v > max_count)
        || total.checked_add(bytes).is_none_or(|v| v > max_bytes)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}

pub(crate) fn read(
    broker: &FsVaultBroker,
    folder: Folder,
    name: &str,
) -> Result<Option<Vec<u8>>, BrokerError> {
    folder.validate_name(name)?;
    let root = parent(broker, folder)?;
    let value = disk::read(&root, name, limits::REQUEST_LIMIT)?;
    store::required(broker)?;
    Ok(value)
}

pub(crate) fn save<T: Serialize>(
    broker: &FsVaultBroker,
    folder: Folder,
    name: &str,
    value: &T,
    expected: Option<&str>,
) -> Result<(), BrokerError> {
    folder.validate_name(name)?;
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > limits::REQUEST_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let root = parent(broker, folder)?;
    let (entries, current_bytes) = scan(broker, folder)?;
    let existing = disk::read_identified(&root, name, limits::REQUEST_LIMIT)?;
    if expected.is_none() && existing.as_ref().is_some_and(|(old, _)| old == &bytes) {
        let (old, identity) = existing.as_ref().ok_or(BrokerError::CorruptWriterState)?;
        disk::FilePin::from_read(old, *identity).check(&root, name, limits::REQUEST_LIMIT)?;
        FsVaultBroker::sync_directory(&root, Path::new("."))?;
        store::required(broker)?;
        return Ok(());
    }
    if existing
        .as_ref()
        .map(|(old, _)| crate::sha256(old))
        .as_deref()
        != expected
    {
        return Err(BrokerError::CaptureChanged);
    }
    let (count, byte_limit) = folder.budget();
    if existing.is_none() && entries.len() >= count {
        return Err(BrokerError::WriterLimitExceeded);
    }
    // 临时副本与旧记录可同时存在：按发布前的实际峰值计费，不能只算替换后的净增量。
    if current_bytes
        .checked_add(bytes.len())
        .is_none_or(|v| v > byte_limit)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let pin = existing
        .as_ref()
        .map(|(bytes, id)| disk::FilePin::from_read(bytes, *id));
    atomic::write(
        broker,
        &root,
        name,
        &bytes,
        &root,
        expected.is_none(),
        || {
            store::required(broker)?;
            if names(broker, folder)? != entries {
                return Err(BrokerError::CaptureChanged);
            }
            if let Some(pin) = &pin {
                pin.check(&root, name, limits::REQUEST_LIMIT)?;
            }
            Ok(())
        },
    )?;
    store::required(broker)?;
    Ok(())
}
