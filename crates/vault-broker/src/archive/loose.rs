//! 散件的读、删和枚举。删之前再核一遍还是读到的那一个（inode 和内容都没变）。
use super::format::Part;
use crate::{
    BrokerError, FsVaultBroker,
    writer::{directory, disk},
};
use cap_std::fs::Dir;
use std::path::Path;

/// 一条散件：读到的原字节，外加删之前要核的身份。
pub(super) struct Loose {
    pub part: Part,
    pub name: String,
    pub bytes: Vec<u8>,
    pin: disk::FilePin,
}

/// 枚举一个目录最多看这么多项。比写入上限宽得多：还原之后散件可能超过写入上限，照样要能收口。
const LIST_LIMIT: usize = 1_000_000;

/// 记录目录；没启用的那些（历史元数据、文件操作）没有目录。
pub(super) fn open(data: &Dir, path: &str) -> Result<Option<Dir>, BrokerError> {
    match data.symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(BrokerError::io("inspect writer record directory", error)),
        Ok(_) => Ok(Some(directory(data, path)?)),
    }
}

pub(super) fn read(data: &Dir, part: Part, name: &str) -> Result<Option<Loose>, BrokerError> {
    let Some(dir) = open(data, part.directory())? else {
        return Ok(None);
    };
    Ok(
        disk::read_identified(&dir, &part.file_name(name), disk::RECORD_LIMIT)?.map(
            |(bytes, identity)| Loose {
                part,
                name: name.to_owned(),
                pin: disk::FilePin::from_read(&bytes, identity),
                bytes,
            },
        ),
    )
}

/// 删掉一条散件；目录由调用方最后一起落盘（[`sync`]）。
pub(super) fn remove(broker: &FsVaultBroker, loose: &Loose) -> Result<(), BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let dir = open(&broker.data, loose.part.directory())?.ok_or(BrokerError::CorruptWriterState)?;
    let file_name = loose.part.file_name(&loose.name);
    loose.pin.check(&dir, &file_name, disk::RECORD_LIMIT)?;
    dir.remove_file(&file_name)
        .map_err(|e| BrokerError::io("remove archived writer record", e))
}

/// 删过散件的目录一起落盘。
pub(super) fn sync(data: &Dir, parts: &[Part]) -> Result<(), BrokerError> {
    let mut paths: Vec<_> = parts.iter().map(|part| part.directory()).collect();
    paths.sort_unstable();
    paths.dedup();
    for path in paths {
        if let Some(dir) = open(data, path)? {
            FsVaultBroker::sync_directory(&dir, Path::new("."))?;
        }
    }
    Ok(())
}

/// 目录里的文件名，排好序；不是正常名字的交给各自的读取函数去拒。
pub(super) fn names(data: &Dir, path: &str) -> Result<Vec<String>, BrokerError> {
    let Some(dir) = open(data, path)? else {
        return Ok(Vec::new());
    };
    let mut names = Vec::new();
    for entry in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate writer records", e))?
    {
        if names.len() >= LIST_LIMIT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect writer records", e))?;
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// 打包会动的四个散件目录。
pub(super) const DIRECTORIES: [&str; 4] = [
    "history/meta-v1/requests",
    "idempotency",
    "file-operations-v1/records",
    "file-operations-v1/acks",
];

/// 这个目录里的这个文件名是哪一条记录；认不出来的不归打包管。
pub(super) fn identify<'a>(path: &str, file_name: &'a str) -> Option<(Part, &'a str)> {
    use crate::file_operation::disk::Folder;
    let found = match path {
        "history/meta-v1/requests" => super::content_part(file_name),
        "idempotency" => file_name
            .strip_suffix(".json")
            .map(|name| (Part::Replay, name)),
        "file-operations-v1/records" => Folder::Records.archived(file_name),
        _ => Folder::Acks.archived(file_name),
    };
    found.filter(|(_, name)| disk::hex(name))
}
