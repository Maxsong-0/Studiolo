//! 写者旧记录的打包存放（BUG-051、T-235）：已封存的请求整组打进不可变段文件，一条不丢。
//!
//! - 一「包」是一个请求的全部记录原字节：正文是描述符、两份审计回执和幂等重放；改名是请求、两份回执和重放。
//!   删除、恢复的文件操作不打包，墓碑从不打包。
//! - 段文件在 `archive-v1/`，内容不可变，逐段连成摘要链；链尾和条数（[`Head`]）写在 `writer-state.json` 里，
//!   存状态就是提交点。段不绑目录的设备号和 inode，恢复到新目录不用重绑定。
//! - 读的时候散件优先，没有散件才查段；两处都有的只能逐字节相同。写入只数散件，所以打包之后照常能写。
//! - 打过包的状态是第 9 版，旧程序不认、拒绝打开；[`unpack`] 把散件还原、状态写回第 8 版之后才能退回旧程序。
mod cutoff;
mod format;
mod index;
mod loose;
mod pack;
mod settle;
mod unpack;
mod usage;

pub use cutoff::{ArchiveBatch, ArchiveCursor, ArchiveCutoff, ArchiveStep};
pub(crate) use format::{Head, Kind, Part};
pub(crate) use index::Archive;
pub(crate) use settle::settle;
pub use unpack::ArchiveUnpack;
pub use usage::{RecordUsage, RecordUsageItem};

use crate::{BrokerError, FsVaultBroker, VaultUuid, records::WriterRecords, writer::disk};
use cap_std::fs::Dir;
use std::sync::Arc;

/// 正文请求的那几份记录：`requests/` 下的文件名归哪一种、属于哪个请求。认不出来的是 `None`。
pub(crate) fn content_part(name: &str) -> Option<(Part, &str)> {
    for (suffix, part) in [
        ("-intent.json.ack", Part::IntentAck),
        ("-outcome.json.ack", Part::OutcomeAck),
        (".json", Part::Descriptor),
    ] {
        if let Some(id) = name.strip_suffix(suffix) {
            return disk::hex(id).then_some((part, id));
        }
    }
    None
}

/// 散件里没有的记录再查段；没有打包过就是 `None`。
pub(crate) fn read(
    records: &dyn WriterRecords,
    part: Part,
    id: &str,
) -> Result<Option<Vec<u8>>, BrokerError> {
    records.archive()?.read(records.data(), part, id)
}

/// 写者打开时把段读进来：状态里的段头说有几段，就得正好有那条链。不在链上的段留给 [`settle`]。
pub(crate) fn open(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let loaded = index::load(
        &broker.data,
        broker.writer.binding.pair_id,
        broker.vault_uuid,
        head(&broker.data, broker.writer.binding.pair_id)?.as_ref(),
    )?;
    install(broker, loaded.archive)
}

/// 封存副本、恢复目录用：只认正好一条链，多一段也不行。
pub(crate) fn exact(data: &Dir, pair: VaultUuid, vault: VaultUuid) -> Result<Archive, BrokerError> {
    let loaded = index::load(data, pair, vault, head(data, pair)?.as_ref())?;
    if loaded.orphans.is_empty() {
        Ok(loaded.archive)
    } else {
        Err(BrokerError::CorruptWriterState)
    }
}

/// 写者状态里的段头。直接读文件：读记录的函数都要先有段，不能绕回 `mutation::load`。
fn head(data: &Dir, pair: VaultUuid) -> Result<Option<Head>, BrokerError> {
    let bytes = disk::read(data, "writer-state.json", disk::RECORD_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    let state: crate::mutation::State = disk::decode(&bytes)?;
    state.validate(pair)?;
    Ok(state.archive)
}

fn install(broker: &FsVaultBroker, archive: Archive) -> Result<(), BrokerError> {
    *broker
        .archive
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)? = Arc::new(archive);
    Ok(())
}

#[cfg(test)]
mod tests;
