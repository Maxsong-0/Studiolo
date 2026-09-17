//! typed 元数据发布只使用同目录 NOREPLACE/rename；不继承旧 helper 的临时硬链接窗口。
use super::{limits, store};
use crate::{BrokerError, FsVaultBroker, VaultUuid, writer::Identity};
use cap_std::fs::{Dir, MetadataExt as _, OpenOptions};
use std::{io::Write, path::Path};

pub(crate) const TEMP_LIMIT: usize = 64;
const PREFIX: &str = ".studiolo-file-operation-";

pub(crate) fn temporary(name: &str) -> Result<bool, BrokerError> {
    let Some(value) = name.strip_prefix(PREFIX) else {
        return Ok(false);
    };
    let id = value
        .strip_suffix(".tmp")
        .filter(|v| v.len() == 36)
        .ok_or(BrokerError::CorruptWriterState)?;
    if id
        .parse::<VaultUuid>()
        .map_err(|_| BrokerError::CorruptWriterState)?
        .to_string()
        != id
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(true)
}

pub(crate) fn write(
    broker: &FsVaultBroker,
    root: &Dir,
    name: &str,
    bytes: &[u8],
    output: &Dir,
    create: bool,
    before_install: impl FnOnce() -> Result<(), BrokerError>,
) -> Result<(), BrokerError> {
    if bytes.len() > limits::REQUEST_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err(BrokerError::WriterLimitExceeded);
    let mut count = 0;
    let mut total = bytes.len();
    for (index, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("enumerate typed metadata temporaries", e))?
        .enumerate()
    {
        if index >= limits::RECORD_COUNT * 2 + TEMP_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect typed metadata temporary", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !temporary(&name)? {
            continue;
        }
        count += 1;
        let metadata = root
            .symlink_metadata(&name)
            .map_err(|e| BrokerError::io("stat typed metadata temporary", e))?;
        FsVaultBroker::validate_file_metadata(&crate::blob::store::logical(&name)?, &metadata)?;
        if metadata.len() > limits::REQUEST_LIMIT as u64
            || metadata.nlink() != 1
            || count >= TEMP_LIMIT
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
        total = total
            .checked_add(metadata.len() as usize)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > TEMP_LIMIT * limits::REQUEST_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    let filename = format!("{PREFIX}{}.tmp", VaultUuid::new());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = root
        .open_with(&filename, &options)
        .map_err(|e| BrokerError::io("create typed metadata temporary", e))?;
    let identity = Identity::file(
        &file
            .metadata()
            .map_err(|e| BrokerError::io("stat typed metadata output", e))?,
    );
    let result = (|| {
        file.write_all(bytes)
            .map_err(|e| BrokerError::io("write typed metadata", e))?;
        file.sync_all()
            .map_err(|e| BrokerError::io("sync typed metadata", e))?;
        before_install()?;
        let named = root
            .symlink_metadata(&filename)
            .map_err(|e| BrokerError::io("recheck typed metadata temporary", e))?;
        FsVaultBroker::validate_file_metadata(&crate::blob::store::logical(&filename)?, &named)?;
        if Identity::file(&named) != identity || named.len() != bytes.len() as u64 {
            return Err(BrokerError::CaptureChanged);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if create {
            rustix::fs::renameat_with(
                root,
                &filename,
                output,
                name,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| BrokerError::io("publish typed metadata", e.into()))?;
        } else {
            root.rename(&filename, output, name)
                .map_err(|e| BrokerError::io("replace typed metadata", e))?;
        }
        FsVaultBroker::sync_directory(root, Path::new("."))?;
        FsVaultBroker::sync_directory(output, Path::new("."))?;
        store::required(broker)?;
        Ok(())
    })();
    if result.is_err()
        && broker.writer.check(&broker.root, &broker.data).is_ok()
        && root
            .symlink_metadata(&filename)
            .is_ok_and(|m| Identity::file(&m) == identity && m.nlink() == 1)
    {
        let _ = root.remove_file(&filename);
        let _ = FsVaultBroker::sync_directory(root, Path::new("."));
    }
    result
}
