//! 在首个 Issued 之前预留两路径全部终态/六角色保全；不依赖去重或稀疏文件。
use super::{
    disk::{self, Folder},
    frozen::Frozen,
    limits,
};
use crate::{
    BrokerError, FsVaultBroker, blob,
    writer::{directory, disk as codec},
};
use cap_std::fs::{Dir, MetadataExt as _};
const META: u64 = 16 * limits::REQUEST_LIMIT as u64;

pub(crate) fn reservation(body: u64, executable: bool) -> Result<u64, BrokerError> {
    body.checked_mul(3)
        .and_then(|v| {
            v.checked_add(if executable {
                7 * blob::OBJECT_LIMIT
            } else {
                0
            })
        })
        .and_then(|v| v.checked_add(META))
        .ok_or(BrokerError::WriterLimitExceeded)
}
pub(crate) fn reserve(
    broker: &FsVaultBroker,
    body: u64,
    executable: bool,
) -> Result<u64, BrokerError> {
    let roots = super::store::required(broker)?;
    if body > roots.limits.effective_object_limit {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let reserve = reservation(body, executable)?;
    let (actual, count) = super::usage::actual(broker)?;
    if actual
        .checked_add(reserve)
        .is_none_or(|v| v > roots.limits.retained_bytes_limit)
        || (executable && count >= roots.limits.max_tombstones as usize)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    for (folder, count) in [
        (Folder::Records, 1),
        (Folder::Acks, 2),
        (Folder::Evidence, 1),
        (Folder::Tombstones, 1),
    ] {
        disk::reserve(broker, folder, count, 4 * limits::REQUEST_LIMIT)?;
    }
    let (_, count, bytes) =
        crate::mutation::replay::scan(&broker.data, Some(broker.writer.binding.pair_id))?;
    if count >= codec::REPLAY_COUNT || bytes > codec::REPLAY_BYTES - codec::RECORD_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    if executable {
        future(broker, body)?;
    }
    super::capacity::free(broker, reserve)?;
    Ok(reserve)
}
// 六个完整发布名和一次复制临时名都预留；不能只逐份复制到目录耗尽才拒绝。
pub(crate) fn future(broker: &FsVaultBroker, body: u64) -> Result<(), BrokerError> {
    slots(
        &directory(&broker.data, "snapshots")?,
        7,
        blob::OBJECT_LIMIT,
    )?;
    slots(&directory(&broker.data, "history")?, 3, body)?;
    blob::quota::reserve(broker, body)?;
    Ok(())
}
pub(crate) fn before_issue(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    slots(
        &directory(&broker.data, "snapshots")?,
        7,
        blob::OBJECT_LIMIT,
    )
}
fn slots(root: &Dir, incoming: usize, largest: u64) -> Result<(), BrokerError> {
    let mut temp_count = 0usize;
    let mut bytes = largest;
    for (count, entry) in root
        .entries()
        .map_err(|e| BrokerError::io("核对重命名保全项数", e))?
        .enumerate()
    {
        if count >= 50_000 - incoming {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("读取重命名保全目录", e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(BrokerError::CorruptWriterState);
        };
        if !name.starts_with(".studiolo-blob-") {
            continue;
        }
        temp_count += 1;
        let metadata = root
            .symlink_metadata(name)
            .map_err(|e| BrokerError::io("核对保全临时项", e))?;
        FsVaultBroker::validate_file_metadata(&blob::store::logical(name)?, &metadata)?;
        bytes = bytes
            .checked_add(metadata.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if temp_count >= 64
            || metadata.nlink() != 1
            || metadata.len() > blob::OBJECT_LIMIT
            || bytes > blob::STAGE_BYTES
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    Ok(())
}
pub(crate) fn layout(frozen: &Frozen) -> Result<(), BrokerError> {
    let super::FileOperationKind::Rename { to, .. } = &frozen.kind else {
        return Err(BrokerError::CorruptWriterState);
    };
    let mut total = 0usize;
    for path in [&frozen.path, to] {
        let mut prefix = String::new();
        for part in path.as_str().split('/') {
            let size = serde_json::to_vec(&prefix)
                .map_err(|_| BrokerError::CorruptWriterState)?
                .len();
            total = total
                .checked_add(size + 512)
                .ok_or(BrokerError::WriterLimitExceeded)?;
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
        }
    }
    let frozen_bytes = serde_json::to_vec(frozen)
        .map_err(|_| BrokerError::CorruptWriterState)?
        .len();
    // 原 Active、终态副本、六角色 review 同时出现在维护槽，按各路径最坏父链重复计费。
    let max = total
        .checked_add(1024)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(frozen_bytes * 4 + 16 * 1024))
        .ok_or(BrokerError::WriterLimitExceeded)?;
    if max > limits::REQUEST_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
