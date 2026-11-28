//! 正文请求目录和重放目录的散件用量（T-249 G5）：写入路径判上限读它，不再每次全目录扫。
//!
//! - 开库之后第一次用到时数一遍，顺带核每个文件名和元数据（和原来每次写入时核的一样）；数不成不留，下次再数。
//! - 之后每写过、删过一条，按那一个文件在盘上现在的样子改一处（[`FsVaultBroker::loose_touch`]）；
//!   哪一步没确认落没落盘，整份作废（[`FsVaultBroker::loose_forget`]），下次用到再数。
//! - 封存、健康页、打包照旧以盘上为准：全目录数、全目录核。每次封存放开之后整份作废、重数一次（自愈，不报损坏：
//!   这份数只用来判上限，报损坏等于拒一切写入直到重启）。用例构建里每次用都和盘上重数一遍比，记漏一处就红。
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    writer::{directory, disk},
};
use cap_std::fs::Dir;
use std::{collections::HashMap, sync::PoisonError};

/// 数到这么多项还没完就不数了：比任何上限都宽，还原之后超过写入上限的照样数得出来。
const LIST_LIMIT: usize = 1_000_000;

/// 哪个散件目录。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LooseFolder {
    /// `history/meta-v1/requests`：正文描述符和两份回执。
    Requests,
    /// `idempotency`：幂等重放，正文和文件操作共用。
    Replays,
}
impl LooseFolder {
    fn path(self) -> &'static str {
        match self {
            Self::Requests => "history/meta-v1/requests",
            Self::Replays => "idempotency",
        }
    }
    /// 这个文件名算不算一条「记录」（请求目录里只有描述符算，回执只占目录项）；认不出来的名字是坏的。
    fn record(self, name: &str) -> Result<bool, BrokerError> {
        let (id, record) = match self {
            Self::Requests => match name
                .strip_suffix("-intent.json.ack")
                .or_else(|| name.strip_suffix("-outcome.json.ack"))
            {
                Some(id) => (id, false),
                None => (
                    name.strip_suffix(".json")
                        .ok_or(BrokerError::CorruptWriterState)?,
                    true,
                ),
            },
            Self::Replays => (
                name.strip_suffix(".json")
                    .ok_or(BrokerError::CorruptWriterState)?,
                true,
            ),
        };
        if !disk::hex(id) {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(record)
    }
}

/// 一个目录现在有什么：文件名到字节数。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Files {
    sizes: HashMap<String, u64>,
}

/// 两个目录的散件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LooseUsage {
    requests: Files,
    replays: Files,
}

/// 一个目录的用量。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    /// 目录里一共几项（请求目录含回执）。
    pub entries: usize,
    /// 其中几条是记录（请求目录只数描述符）。
    pub records: usize,
    /// 记录一共几字节（请求目录只算描述符）。
    pub bytes: u64,
}

impl LooseUsage {
    fn files(&mut self, folder: LooseFolder) -> &mut Files {
        match folder {
            LooseFolder::Requests => &mut self.requests,
            LooseFolder::Replays => &mut self.replays,
        }
    }
    /// 这个目录的用量。
    pub(crate) fn tally(&self, folder: LooseFolder) -> Tally {
        let files = match folder {
            LooseFolder::Requests => &self.requests,
            LooseFolder::Replays => &self.replays,
        };
        let mut tally = Tally {
            entries: files.sizes.len(),
            ..Tally::default()
        };
        for (name, bytes) in &files.sizes {
            // 名字建表时核过，这里不会是坏的。
            if folder.record(name).unwrap_or(false) {
                tally.records += 1;
                tally.bytes += bytes;
            }
        }
        tally
    }
}

/// 从盘上数一遍，逐个核名字和元数据。目录还没建的当成空的。
fn scan(broker: &FsVaultBroker) -> Result<LooseUsage, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let mut usage = LooseUsage::default();
    for folder in [LooseFolder::Requests, LooseFolder::Replays] {
        let dir = match broker.data.symlink_metadata(folder.path()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(BrokerError::io("inspect writer record directory", error)),
            Ok(_) => directory(&broker.data, folder.path())?,
        };
        for entry in dir
            .entries()
            .map_err(|e| BrokerError::io("enumerate writer records", e))?
        {
            #[cfg(test)]
            disk::probe::listed();
            if usage.files(folder).sizes.len() >= LIST_LIMIT {
                return Err(BrokerError::WriterLimitExceeded);
            }
            let name = entry
                .map_err(|e| BrokerError::io("inspect writer record", e))?
                .file_name()
                .into_string()
                .map_err(|_| BrokerError::CorruptWriterState)?;
            let bytes = size(&dir, folder, &name)?.ok_or(BrokerError::CorruptWriterState)?;
            usage.files(folder).sizes.insert(name, bytes);
        }
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(usage)
}

/// 一个文件现在多大；不在是 `None`。名字、类型、链接数、大小不对都是坏的。
fn size(dir: &Dir, folder: LooseFolder, name: &str) -> Result<Option<u64>, BrokerError> {
    folder.record(name)?;
    let logical = VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?;
    let metadata = match dir.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BrokerError::io("stat writer record", error)),
    };
    FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
    if metadata.len() > disk::RECORD_LIMIT as u64 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(Some(metadata.len()))
}

impl FsVaultBroker {
    /// 现在的散件用量；还没数过（或作废了）就先数一遍。
    pub(crate) fn loose_usage(&self) -> Result<LooseUsage, BrokerError> {
        let mut cached = self.loose.lock().unwrap_or_else(PoisonError::into_inner);
        let usage = match cached.as_ref() {
            Some(usage) => usage.clone(),
            None => {
                let usage = scan(self)?;
                *cached = Some(usage.clone());
                usage
            }
        };
        drop(cached);
        // 用例构建：每次都和盘上重数的比（不算进读取计数），哪一处增删没记上就在这里红。
        // 盘上数不成（用例故意弄坏了）不比：生产里这一步也看不见，坏的由开库、封存、打包去拒。
        #[cfg(test)]
        {
            if let Ok(fresh) = disk::probe::uncounted(|| scan(self)) {
                assert_eq!(usage, fresh, "散件用量和盘上不一致：有一处增删没记");
            }
        }
        Ok(usage)
    }

    /// 这一条散件刚写过或删过：按盘上现在的样子记（在就记大小，不在就去掉）。看不清就整份作废。
    pub(crate) fn loose_touch(&self, folder: LooseFolder, name: &str) {
        let mut cached = self.loose.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(usage) = cached.as_mut() else {
            return;
        };
        let now = loose_dir(self, folder).and_then(|dir| match dir {
            Some(dir) => size(&dir, folder, name),
            None => Ok(None),
        });
        match now {
            Ok(Some(bytes)) => {
                usage.files(folder).sizes.insert(name.to_owned(), bytes);
            }
            Ok(None) => {
                usage.files(folder).sizes.remove(name);
            }
            Err(_) => *cached = None,
        }
    }

    /// 记不准了：下次用到时从盘上重数。
    pub(crate) fn loose_forget(&self) {
        *self.loose.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn loose_dir(broker: &FsVaultBroker, folder: LooseFolder) -> Result<Option<Dir>, BrokerError> {
    match broker.data.symlink_metadata(folder.path()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(BrokerError::io("inspect writer record directory", error)),
        Ok(_) => Ok(Some(directory(&broker.data, folder.path())?)),
    }
}
