//! 收口：上次打包或还原断在中间留下的，在这里接着做完。只删与另一份逐字节相同的；核不上一律拒绝，什么都不删。
//!
//! - 不在链上的段（段已落、状态没存，或者还原后状态已经写回）：里面每一条散件都在且相同，才删这个段。
//! - 链上的段还留着散件（状态已存、散件没删完）：和段里相同，才删散件。
//! - 一段也没有了（还原完、或者第一段没提交）：空目录一起删，和没打过包的一样。
use super::{Archive, format::DIRECTORY, index, loose};
use crate::{BrokerError, FsVaultBroker, records::WriterRecords, writer::directory};
use cap_std::fs::Dir;
use std::{path::Path, sync::atomic::Ordering};

pub(crate) fn settle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    let archive = broker.archive()?;
    orphans(broker, &archive)?;
    leftovers(broker, &archive)?;
    broker.archive_dirty.store(false, Ordering::Release);
    Ok(())
}

impl FsVaultBroker {
    /// 上一步打包没做完的，现在收口；没有就什么都不做，交回假。打包的触发条件管不着它：
    /// 留着的孤儿段会让封存一直做不成，不能等到下一次该打包的时候。有在途的事就先不收（`MutationUnresolved`）。
    pub fn archive_settle(&mut self) -> Result<bool, BrokerError> {
        if !self.archive_dirty.load(Ordering::Acquire) && !has_orphan(self)? {
            return Ok(false);
        }
        super::pack::idle(self)?;
        settle(self)?;
        Ok(true)
    }
}

/// 目录里有不在链上的段。
fn has_orphan(broker: &FsVaultBroker) -> Result<bool, BrokerError> {
    let Some(names) = index::names(&broker.data)? else {
        return Ok(false);
    };
    let archive = broker.archive()?;
    Ok(names.iter().any(|name| !archive.has_segment(name)))
}

fn orphans(broker: &FsVaultBroker, archive: &Archive) -> Result<(), BrokerError> {
    let Some(names) = index::names(&broker.data)? else {
        return Ok(());
    };
    let dir = directory(&broker.data, DIRECTORY)?;
    let mut removed = false;
    for name in names {
        if archive.has_segment(&name) {
            continue;
        }
        let (_, segment) = index::read_segment(
            &broker.data,
            &name,
            broker.writer.binding.pair_id,
            broker.vault_uuid,
        )?;
        if loose::matching(&broker.data, &segment)?.is_none() {
            return Err(BrokerError::CorruptWriterState);
        }
        broker.writer.check(&broker.root, &broker.data)?;
        dir.remove_file(&name)
            .map_err(|e| BrokerError::io("remove uncommitted writer archive segment", e))?;
        removed = true;
    }
    if removed {
        FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    }
    if archive.head().is_none() && empty(&dir)? {
        broker.writer.check(&broker.root, &broker.data)?;
        broker
            .data
            .remove_dir(DIRECTORY)
            .map_err(|e| BrokerError::io("remove writer archive", e))?;
        FsVaultBroker::sync_directory(&broker.data, Path::new("."))?;
    }
    Ok(())
}

/// 里面什么都没有。没挪走的原子写残留也算有：目录留着，导出扫描照旧拒绝。
fn empty(dir: &Dir) -> Result<bool, BrokerError> {
    Ok(dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate writer archive", e))?
        .next()
        .is_none())
}

fn leftovers(broker: &FsVaultBroker, archive: &Archive) -> Result<(), BrokerError> {
    if archive.head().is_none() {
        return Ok(());
    }
    let mut kept = Vec::new();
    for path in loose::DIRECTORIES {
        for file_name in loose::names(&broker.data, path)? {
            let Some((part, name)) = loose::identify(path, &file_name) else {
                continue;
            };
            let Some(bytes) = archive.read(&broker.data, part, name)? else {
                continue;
            };
            let found = loose::read(&broker.data, part, name)?
                .filter(|found| found.bytes == bytes)
                .ok_or(BrokerError::CorruptWriterState)?;
            kept.push(found);
        }
    }
    // 全部核过才开始删。
    for found in &kept {
        loose::remove(broker, found)?;
    }
    let parts: Vec<_> = kept.iter().map(|found| found.part).collect();
    loose::sync(&broker.data, &parts)
}
