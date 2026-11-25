//! 收口：上次打包或还原断在中间留下的，在这里接着做完。只删与另一份逐字节相同的；核不上一律拒绝，什么都不删。
//!
//! - 不在链上的段（段已落、状态没存，或者还原后状态已经写回）：里面每一条散件都在且相同，才删这个段。
//! - 链上的段还留着散件（状态已存、散件没删完）：和段里相同，才删散件。
use super::{Archive, format::DIRECTORY, index, loose};
use crate::{BrokerError, FsVaultBroker, records::WriterRecords, writer::directory};
use std::{path::Path, sync::atomic::Ordering};

pub(crate) fn settle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    let archive = broker.archive()?;
    orphans(broker, &archive)?;
    leftovers(broker, &archive)?;
    broker.archive_dirty.store(false, Ordering::Release);
    Ok(())
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
        for (pack, _) in &segment.packs {
            for (part, bytes) in pack.kind.parts().into_iter().zip(&pack.bytes) {
                let kept = loose::read(&broker.data, part, pack.name_of(part))?;
                if kept.is_none_or(|kept| &kept.bytes != bytes) {
                    return Err(BrokerError::CorruptWriterState);
                }
            }
        }
        broker.writer.check(&broker.root, &broker.data)?;
        dir.remove_file(&name)
            .map_err(|e| BrokerError::io("remove uncommitted writer archive segment", e))?;
        removed = true;
    }
    if removed {
        FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    }
    Ok(())
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
