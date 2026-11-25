//! 还原：把段里的记录写回散件、状态写回第 8 版、删段。停人侧之后由维护命令调，给退回旧程序用。
use super::{
    Archive,
    loose::{self, Loose},
    pack,
};
use crate::{
    BrokerError, FsVaultBroker, VaultPath, mutation, records::WriterRecords, writer::disk,
};
use std::{path::Path, sync::atomic::Ordering};

/// 还原的结果。散件超过旧上限时旧程序照样能起、旧键照样答得出，但新写入和封存会照旧被拒。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ArchiveUnpack {
    /// 还原了几包。
    pub packs: u64,
    /// 还原之后正文描述符几条。
    pub descriptors: u64,
    /// 还原之后幂等重放几条。
    pub replays: u64,
    /// 还原之后文件操作请求几条。
    pub file_operations: u64,
    /// 有一处超过旧程序的写入上限。
    pub over_limit: bool,
}

impl FsVaultBroker {
    /// 还原之后会是什么样；只读，给维护命令先摆给操作者看。
    pub fn archive_unpack_preview(&self) -> Result<ArchiveUnpack, BrokerError> {
        pack::idle(self)?;
        report(self, &*self.archive()?)
    }
    /// 还原。断在中间可以再跑，也可以正常打开当作打包没删完收口。
    pub fn archive_unpack(&mut self) -> Result<ArchiveUnpack, BrokerError> {
        pack::idle(self)?;
        // 上次断在「状态已写回、段没删」的，这里接着删完，连空目录一起。
        super::settle(self)?;
        let archive = self.archive()?;
        let expected = report(self, &archive)?;
        if archive.head().is_none() {
            return Ok(expected);
        }
        self.archive_dirty.store(true, Ordering::Release);
        restore(self, &archive)?;
        mutation::fault(self, 107)?;
        let mut state = mutation::load(self)?;
        state.archive = None;
        state.format_version = 8;
        mutation::save(self, &state)?;
        super::install(self, Archive::default())?;
        // 状态已经写回：段是不在链上的，散件全在，收口会核过再删。
        mutation::fault(self, 108)?;
        super::settle(self)?;
        Ok(expected)
    }
}

fn restore(broker: &FsVaultBroker, archive: &Archive) -> Result<(), BrokerError> {
    let total = archive.packs().len();
    for (index, pack) in archive.packs().iter().enumerate() {
        for part in pack.kind.parts() {
            let name = pack.name_of(part);
            let bytes = archive
                .read(&broker.data, part, name)?
                .ok_or(BrokerError::CorruptWriterState)?;
            if let Some(Loose { bytes: kept, .. }) = loose::read(&broker.data, part, name)? {
                if kept != bytes {
                    return Err(BrokerError::CorruptWriterState);
                }
                continue;
            }
            let dir = loose::open(&broker.data, part.directory())?
                .ok_or(BrokerError::CorruptWriterState)?;
            let file_name = part.file_name(name);
            FsVaultBroker::atomic_write_dir_checked(
                &dir,
                Path::new(&file_name),
                &bytes,
                true,
                || broker.writer.check(&broker.root, &broker.data),
            )?;
            let logical =
                VaultPath::parse(&file_name).map_err(|_| BrokerError::CorruptWriterState)?;
            if disk::read(&dir, logical.as_str(), disk::RECORD_LIMIT)?.as_ref() != Some(&bytes) {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        if index == total / 2 {
            mutation::fault(broker, 106)?;
        }
    }
    Ok(())
}

fn report(broker: &FsVaultBroker, archive: &Archive) -> Result<ArchiveUnpack, BrokerError> {
    let usage = broker.record_usage()?;
    let content = archive
        .packs()
        .iter()
        .filter(|pack| pack.kind == super::format::Kind::Content)
        .count() as u64;
    let packs = archive.packs().len() as u64;
    let value = ArchiveUnpack {
        packs,
        descriptors: usage.descriptors.used + content,
        replays: usage.replays.used + packs,
        file_operations: usage.file_operations.used + (packs - content),
        over_limit: false,
    };
    Ok(ArchiveUnpack {
        over_limit: value.descriptors > usage.descriptors.limit
            || value.replays > usage.replays.limit
            || value.file_operations > usage.file_operations.limit,
        ..value
    })
}
