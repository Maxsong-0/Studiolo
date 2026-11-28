//! 实际保全字节与可释放的未决预留分开；只统计固定私有域，不清理任何已发布对象。
use super::{
    disk::{self, Folder},
    limits,
};
use crate::{
    BrokerError, FsVaultBroker,
    blob::store as blobs,
    records::WriterRecords,
    writer::{directory, disk as codec},
};
use cap_std::fs::{Dir, MetadataExt as _};
use std::collections::BTreeSet;

pub(crate) fn actual(broker: &FsVaultBroker) -> Result<(u64, usize), BrokerError> {
    let rename_enabled = crate::mutation::load(broker)?.rename_enabled;
    let mut retained = 0u64;
    let mut revisions = BTreeSet::new();
    let mut candidates = 0;
    // 散件和打了包的同一套判法：打了包的照样占墓碑名额、照样保护 revision（T-249）。
    for record in super::retained::all(broker)? {
        if record.occupies() {
            candidates += 1;
        }
        for revision in record.revisions(broker)? {
            revisions.insert(revision.to_string());
        }
    }
    // 打了包的请求和回执照样计字节。
    retained = add(retained, broker.archive()?.file_bytes())?;
    let roots = super::store::required(broker)?;
    retained = add(
        retained,
        scan(&roots.quarantine, limits::RECORD_COUNT, |name| {
            if !codec::hex(name.strip_suffix(".object").unwrap_or("")) {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(true)
        })?,
    )?;
    let history = directory(&broker.data, "history")?;
    retained = add(
        retained,
        scan(&history, 50_001, |name| {
            Ok(revisions.contains(name) || blob_temporary(name)?)
        })?,
    )?;
    let snapshots = directory(&broker.data, "snapshots")?;
    retained = add(
        retained,
        scan(&snapshots, 50_001, |name| {
            if blob_temporary(name)? {
                return Ok(true);
            }
            let Some(value) = name.strip_prefix("file-operation-") else {
                return Ok(false);
            };
            let Some((id, role)) = value.split_once('-') else {
                return Err(BrokerError::CorruptWriterState);
            };
            if !rename_enabled
                && matches!(
                    role,
                    "source_current.bin"
                        | "target_current.bin"
                        | "source_original.bin"
                        | "target_proposed.bin"
                        | "source_quarantine.bin"
                        | "target_stage.bin"
                )
            {
                return Err(BrokerError::CorruptWriterState);
            }
            if !codec::hex(id)
                || !matches!(
                    role,
                    "current.bin"
                        | "history.bin"
                        | "quarantine.bin"
                        | "stage.bin"
                        | "source_current.bin"
                        | "target_current.bin"
                        | "source_original.bin"
                        | "target_proposed.bin"
                        | "source_quarantine.bin"
                        | "target_stage.bin"
                )
            {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(true)
        })?,
    )?;
    let stages = directory(&broker.root, ".workbench/staging")?;
    retained = add(
        retained,
        scan(&stages, crate::blob::STAGE_COUNT, |name| {
            if !codec::hex(name.strip_suffix(".stage").unwrap_or("")) {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(true)
        })?,
    )?;
    for folder in [
        Folder::Records,
        Folder::Tombstones,
        Folder::Acks,
        Folder::Evidence,
    ] {
        retained = add(retained, disk::usage(broker, folder)? as u64)?;
    }
    Ok((retained, candidates))
}
fn scan(
    root: &Dir,
    maximum: usize,
    selected: impl Fn(&str) -> Result<bool, BrokerError>,
) -> Result<u64, BrokerError> {
    let mut bytes = 0;
    for (index, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("enumerate retained file operation objects", e))?
        .enumerate()
    {
        if index >= maximum {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect retained object", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !selected(&name)? {
            continue;
        }
        let metadata = root
            .symlink_metadata(&name)
            .map_err(|e| BrokerError::io("stat retained object", e))?;
        FsVaultBroker::validate_file_metadata(&blobs::logical(&name)?, &metadata)?;
        if metadata.nlink() != 1 || metadata.len() > crate::blob::OBJECT_LIMIT {
            return Err(BrokerError::CorruptWriterState);
        }
        bytes = add(bytes, metadata.len())?;
    }
    Ok(bytes)
}
fn blob_temporary(name: &str) -> Result<bool, BrokerError> {
    let Some(value) = name.strip_prefix(".studiolo-blob-") else {
        return Ok(false);
    };
    let id = value
        .strip_suffix(".tmp")
        .ok_or(BrokerError::CorruptWriterState)?;
    if id
        .parse::<crate::VaultUuid>()
        .map_err(|_| BrokerError::CorruptWriterState)?
        .to_string()
        != id
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(true)
}
fn add(left: u64, right: u64) -> Result<u64, BrokerError> {
    left.checked_add(right)
        .ok_or(BrokerError::WriterLimitExceeded)
}
