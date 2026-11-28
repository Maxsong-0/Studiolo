//! 不扫无限请求目录：条数、单文件和总字节都有上限，并为两份 ACK 留出空间。
//!
//! 写入路径读内存里的散件用量（`loose_usage`），不再每次全目录扫（T-249 G5）。描述符和重放用同一把尺：
//! 一次正文写入要多一条描述符、两份回执、一条重放，哪一处放不下都在落盘之前拒绝，不让重放越过封存的上限（G1）。

use crate::{
    BrokerError, FsVaultBroker,
    loose_usage::{LooseFolder, Tally},
    writer::disk,
};

/// 全部描述符（加上给回执留的空）最多这么多字节。
pub(crate) const DESCRIPTOR_BYTES: usize = 32 * 1024 * 1024;
/// 每条描述符给两份回执留的空。
pub(crate) const ACK_BYTES: usize = 2048;

pub(crate) fn reserve(broker: &FsVaultBroker, incoming: usize) -> Result<(), BrokerError> {
    if crate::history::store::open(broker)?.is_none() {
        return Ok(());
    }
    let usage = broker.loose_usage()?;
    let requests = usage.tally(LooseFolder::Requests);
    let bytes = (requests.bytes as usize)
        .checked_add(incoming)
        .ok_or(BrokerError::WriterLimitExceeded)?;
    if requests.entries > 3 * disk::REPLAY_COUNT
        || requests.records >= disk::REPLAY_COUNT
        || bytes.saturating_add((requests.records + 1) * ACK_BYTES) > DESCRIPTOR_BYTES
        || !replay_room(usage.tally(LooseFolder::Replays))
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    crate::history::store::open(broker)?;
    Ok(())
}

/// 重放目录还放得下一条：正文写入、文件操作开始之前都按它判，和封存核对的上限一致。
pub(crate) fn replay_room(replays: Tally) -> bool {
    replays.records < disk::REPLAY_COUNT
        && replays.bytes as usize <= disk::REPLAY_BYTES - disk::RECORD_LIMIT
}
