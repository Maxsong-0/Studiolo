//! 已确认终态释放未使用的预留；真实 L0/隔离/证据与残留始终按固定域实际字节计费。
use super::{
    disk::{self, Folder},
    limits, store,
};
use crate::{BrokerError, FsVaultBroker, writer::disk as codec};

pub(crate) fn reserve(
    broker: &FsVaultBroker,
    body: u64,
    deleting: bool,
    executable: bool,
) -> Result<u64, BrokerError> {
    let roots = store::required(broker)?;
    if body > roots.limits.effective_object_limit {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let reservation = body
        .checked_mul(if deleting { 2 } else { 1 })
        .and_then(|v| v.checked_add(4 * limits::REQUEST_LIMIT as u64))
        .and_then(|v| {
            v.checked_add(if executable {
                3 * crate::blob::OBJECT_LIMIT
            } else {
                0
            })
        })
        .ok_or(BrokerError::WriterLimitExceeded)?;
    let (retained, candidates) = super::usage::actual(broker)?;
    if retained
        .checked_add(reservation)
        .is_none_or(|v| v > roots.limits.retained_bytes_limit)
        || (deleting && candidates >= roots.limits.max_tombstones as usize)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    for folder in [Folder::Records, Folder::Acks, Folder::Evidence] {
        disk::reserve(
            broker,
            folder,
            if matches!(folder, Folder::Acks) { 2 } else { 1 },
            4 * limits::REQUEST_LIMIT,
        )?;
    }
    if deleting {
        disk::reserve(broker, Folder::Tombstones, 1, 2 * limits::REQUEST_LIMIT)?;
    }
    let (_, count, bytes) =
        crate::mutation::replay::scan(&broker.data, Some(broker.writer.binding.pair_id))?;
    if count >= codec::REPLAY_COUNT || bytes > codec::REPLAY_BYTES - codec::RECORD_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    free(
        broker,
        if executable {
            crate::blob::OBJECT_LIMIT * 3 + body
        } else {
            0
        },
    )?;
    Ok(reservation)
}
pub(crate) fn free(broker: &FsVaultBroker, body: u64) -> Result<(), BrokerError> {
    let limit = store::required(broker)?.limits;
    let needed = limit
        .minimum_free_bytes
        .checked_add(body)
        .and_then(|v| v.checked_add(1024 * 1024))
        .ok_or(BrokerError::WriterLimitExceeded)?;
    for root in [&broker.root, &broker.data] {
        let stat = rustix::fs::fstatvfs(root)
            .map_err(|e| BrokerError::io("check file operation free capacity", e.into()))?;
        if stat
            .f_bavail
            .checked_mul(stat.f_frsize)
            .is_none_or(|v| v < needed)
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    Ok(())
}

// 同一路径的未来 current/执行证明与维护事件必须始终容纳于旧 64 KiB 主/维护日志。
// 使用整数与转义字符串的最坏编码长度，不依赖本次较短的 inode/时间数字。
pub(crate) fn layout(frozen: &super::frozen::Frozen) -> Result<(), BrokerError> {
    let mut prefix = String::new();
    let mut parents = 0usize;
    for part in frozen.path.as_str().split('/') {
        let encoded = serde_json::to_vec(&prefix).map_err(|_| BrokerError::CorruptWriterState)?;
        parents = parents
            .checked_add(encoded.len() + 512)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
    }
    let frozen_bytes = serde_json::to_vec(frozen)
        .map_err(|_| BrokerError::CorruptWriterState)?
        .len();
    let maximum = parents
        .checked_add(1024)
        .and_then(|v| v.checked_mul(4))
        .and_then(|v| v.checked_add(frozen_bytes * 3 + 16 * 1024))
        .ok_or(BrokerError::WriterLimitExceeded)?;
    if maximum > limits::REQUEST_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
