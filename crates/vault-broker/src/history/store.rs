//! 版本化私有元数据目录；旧字节保持原名，未知或半初始化 schema 失败关闭。

use crate::writer::{Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, VaultUuid};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use std::path::Path;

const NAME: &str = "meta-v1";
pub(crate) const CATALOG_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 4096;
pub(crate) const MAX_OUTBOX: usize = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) version: u32,
    pub(super) pair_id: VaultUuid,
    pub(super) vault_uuid: VaultUuid,
    pub(super) root: Identity,
    pub(super) requests: Identity,
    pub(super) catalogs: Identity,
    pub(super) outbox: Identity,
}

pub(crate) struct Pin {
    binding: Binding,
    file: disk::FilePin,
}

pub(crate) fn open(broker: &FsVaultBroker, create: bool) -> Result<Option<Dir>, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let history = directory(&broker.data, "history")?;
    let mut pin = broker
        .history_pin
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    match history.symlink_metadata(NAME) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if pin.is_some() {
                return Err(BrokerError::WriterBindingMismatch);
            }
            if !create {
                return Ok(None);
            }
            super::schema::initialize(broker, &history)?;
        }
        Err(error) => return Err(BrokerError::io("inspect history metadata", error)),
        Ok(_) => {}
    }
    let dir = directory(&history, NAME)?;
    if pin.is_none() {
        let (bytes, identity) = disk::read_identified(&dir, "binding.json", disk::IDENTITY_LIMIT)?
            .ok_or(BrokerError::CorruptWriterState)?;
        let binding: Binding = disk::decode(&bytes)?;
        validate(broker, &dir, &binding)?;
        *pin = Some(Pin {
            binding,
            file: disk::FilePin::from_read(&bytes, identity),
        });
    }
    let pin = pin.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    pin.file.check(&dir, "binding.json", disk::IDENTITY_LIMIT)?;
    validate(broker, &dir, &pin.binding)?;
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(Some(dir))
}

fn validate(broker: &FsVaultBroker, dir: &Dir, binding: &Binding) -> Result<(), BrokerError> {
    if binding.version != 1
        || binding.pair_id != broker.writer.binding.pair_id
        || binding.vault_uuid != broker.vault_uuid
        || Identity::directory(dir)? != binding.root
    {
        return Err(BrokerError::WriterBindingMismatch);
    }
    for (name, identity) in [
        ("requests", binding.requests),
        ("catalogs", binding.catalogs),
        ("outbox", binding.outbox),
    ] {
        if Identity::directory(&directory(dir, name)?)? != identity {
            return Err(BrokerError::WriterBindingMismatch);
        }
    }
    Ok(())
}

pub(crate) fn read(
    broker: &FsVaultBroker,
    name: &str,
    max: usize,
) -> Result<Option<Vec<u8>>, BrokerError> {
    let Some(dir) = open(broker, false)? else {
        return Ok(None);
    };
    let result = disk::read(&dir, name, max)?;
    open(broker, false)?;
    Ok(result)
}

pub(crate) fn save<T: Serialize>(
    broker: &FsVaultBroker,
    name: &str,
    value: &T,
    max: usize,
    create: bool,
) -> Result<(), BrokerError> {
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > max {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let dir = open(broker, true)?.ok_or(BrokerError::CorruptWriterState)?;
    let (folder, basename) = name
        .split_once('/')
        .ok_or(BrokerError::CorruptWriterState)?;
    if !matches!(folder, "requests" | "catalogs" | "outbox") || basename.contains('/') {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = directory(&dir, folder)?;
    let identity = Identity::directory(&parent)?;
    FsVaultBroker::atomic_write_dir_checked(&parent, Path::new(basename), &bytes, create, || {
        open(broker, false)?.ok_or(BrokerError::CorruptWriterState)?;
        if Identity::directory(&directory(&dir, folder)?)? != identity {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(())
    })?;
    open(broker, false)?;
    Ok(())
}

pub(crate) fn remove(broker: &FsVaultBroker, name: &str) -> Result<(), BrokerError> {
    let dir = open(broker, false)?.ok_or(BrokerError::CorruptWriterState)?;
    let (folder, basename) = name
        .split_once('/')
        .ok_or(BrokerError::CorruptWriterState)?;
    if !matches!(folder, "requests" | "catalogs" | "outbox") || basename.contains('/') {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = directory(&dir, folder)?;
    parent
        .remove_file(basename)
        .map_err(|e| BrokerError::io("remove history metadata", e))?;
    FsVaultBroker::sync_directory(&parent, Path::new("."))?;
    open(broker, false)?;
    Ok(())
}

pub(crate) fn now() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
}

pub(crate) use super::catalog_disk::save_catalog;
