//! 版本化私有元数据目录；旧字节保持原名，未知或半初始化 schema 失败关闭。

use crate::archive::{self, Part};
use crate::writer::{Identity, directory, disk};
use crate::{BrokerError, FsVaultBroker, VaultUuid, records::WriterRecords};
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

/// 打开已有的元数据目录；没有就是 `None`。
pub(crate) fn open(records: &dyn WriterRecords) -> Result<Option<Dir>, BrokerError> {
    open_with(records, None)
}
/// 只给写者写入前用：没有元数据目录就先建好。
pub(crate) fn open_or_create(broker: &FsVaultBroker) -> Result<Dir, BrokerError> {
    open_with(broker, Some(broker))?.ok_or(BrokerError::CorruptWriterState)
}
fn open_with(
    records: &dyn WriterRecords,
    create: Option<&FsVaultBroker>,
) -> Result<Option<Dir>, BrokerError> {
    records.check()?;
    let history = directory(records.data(), "history")?;
    let mut pin = records
        .history_pin()
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    match history.symlink_metadata(NAME) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if pin.is_some() {
                return Err(BrokerError::WriterBindingMismatch);
            }
            let Some(broker) = create else {
                return Ok(None);
            };
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
        validate(records, &dir, &binding)?;
        *pin = Some(Pin {
            binding,
            file: disk::FilePin::from_read(&bytes, identity),
        });
    }
    let pin = pin.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    pin.file.check(&dir, "binding.json", disk::IDENTITY_LIMIT)?;
    validate(records, &dir, &pin.binding)?;
    records.check()?;
    Ok(Some(dir))
}

fn validate(records: &dyn WriterRecords, dir: &Dir, binding: &Binding) -> Result<(), BrokerError> {
    if binding.version != 1
        || binding.pair_id != records.pair_id()
        || binding.vault_uuid != records.vault_uuid()
        || records.bound_identity(dir)? != binding.root
    {
        return Err(BrokerError::WriterBindingMismatch);
    }
    for (name, identity) in [
        ("requests", binding.requests),
        ("catalogs", binding.catalogs),
        ("outbox", binding.outbox),
    ] {
        if records.bound_identity(&directory(dir, name)?)? != identity {
            return Err(BrokerError::WriterBindingMismatch);
        }
    }
    Ok(())
}

pub(crate) fn read(
    records: &dyn WriterRecords,
    name: &str,
    max: usize,
) -> Result<Option<Vec<u8>>, BrokerError> {
    let Some(dir) = open(records)? else {
        return Ok(None);
    };
    let result = disk::read(&dir, name, max)?;
    open(records)?;
    // 请求的描述符和回执可能已经打包：散件优先，没有再查段。
    match (result, archived(name)) {
        (None, Some((part, id))) => archive::read(records, part, id),
        (result, _) => Ok(result),
    }
}

/// 这个名字是不是一条可能打了包的记录。
fn archived(name: &str) -> Option<(Part, &str)> {
    name.strip_prefix("requests/")
        .and_then(archive::content_part)
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
    // 已经打包的不许再造一份散件；调用方都先读过，到这里说明前后不一致。
    if let Some((part, id)) = archived(name)
        && broker.archive()?.contains(part, id)?
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let dir = open_or_create(broker)?;
    let (folder, basename) = name
        .split_once('/')
        .ok_or(BrokerError::CorruptWriterState)?;
    if !matches!(folder, "requests" | "catalogs" | "outbox") || basename.contains('/') {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = directory(&dir, folder)?;
    let identity = Identity::directory(&parent)?;
    let written = FsVaultBroker::atomic_write_dir_checked(
        &parent,
        Path::new(basename),
        &bytes,
        create,
        || {
            open(broker)?.ok_or(BrokerError::CorruptWriterState)?;
            if Identity::directory(&directory(&dir, folder)?)? != identity {
                return Err(BrokerError::WriterBindingMismatch);
            }
            Ok(())
        },
    );
    // 写成没写成都按盘上现在的样子记（T-249 G5）。
    if folder == "requests" {
        broker.loose_touch(crate::loose_usage::LooseFolder::Requests, basename);
    }
    written?;
    open(broker)?;
    Ok(())
}

pub(crate) fn remove(broker: &FsVaultBroker, name: &str) -> Result<(), BrokerError> {
    let dir = open(broker)?.ok_or(BrokerError::CorruptWriterState)?;
    let (folder, basename) = name
        .split_once('/')
        .ok_or(BrokerError::CorruptWriterState)?;
    if !matches!(folder, "requests" | "catalogs" | "outbox") || basename.contains('/') {
        return Err(BrokerError::CorruptWriterState);
    }
    let parent = directory(&dir, folder)?;
    let removed = parent
        .remove_file(basename)
        .map_err(|e| BrokerError::io("remove history metadata", e));
    // 现在只删外发箱里的；请求目录里的也删时照样记（T-249 G5）。
    if folder == "requests" {
        broker.loose_touch(crate::loose_usage::LooseFolder::Requests, basename);
    }
    removed?;
    FsVaultBroker::sync_directory(&parent, Path::new("."))?;
    open(broker)?;
    Ok(())
}

pub(crate) fn now() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
}

pub(crate) use super::catalog_disk::save_catalog;
