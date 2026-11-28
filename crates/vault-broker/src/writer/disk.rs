//! 此协议接触的文件均有字节上限，并拒绝链接、特殊文件和读取期间替换。

use super::Identity;
use crate::{BrokerError, FsVaultBroker, VaultPath, sha256};
use cap_std::fs::{Dir, MetadataExt as _};
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Read, path::Path};

pub(crate) const IDENTITY_LIMIT: usize = 4096;
pub(crate) const RECORD_LIMIT: usize = 64 * 1024;
pub(crate) const CONTENT_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const REPLAY_COUNT: usize = 4096;
pub(crate) const REPLAY_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct FilePin {
    identity: Identity,
    digest: String,
}

impl FilePin {
    pub(crate) fn from_read(bytes: &[u8], identity: Identity) -> Self {
        Self {
            identity,
            digest: sha256(bytes),
        }
    }
    pub(crate) fn capture(dir: &Dir, path: &str, max: usize) -> Result<Self, BrokerError> {
        let (bytes, identity) =
            read_identified(dir, path, max)?.ok_or(BrokerError::CorruptWriterState)?;
        Ok(Self::from_read(&bytes, identity))
    }
    pub(crate) fn check(&self, dir: &Dir, path: &str, max: usize) -> Result<Vec<u8>, BrokerError> {
        let (bytes, identity) =
            read_identified(dir, path, max)?.ok_or(BrokerError::WriterBindingMismatch)?;
        if identity != self.identity || sha256(&bytes) != self.digest {
            return Err(BrokerError::WriterBindingMismatch);
        }
        Ok(bytes)
    }
}

/// 用例用：数这个线程上读了几个记录文件、枚举了几项记录目录。写入路径每次读多少不该随散件条数涨（T-249）。
#[cfg(test)]
pub(crate) mod probe {
    use std::cell::Cell;
    thread_local! {
        static READS: Cell<u64> = const { Cell::new(0) };
        static LISTED: Cell<u64> = const { Cell::new(0) };
    }
    pub(crate) fn read() {
        READS.with(|count| count.set(count.get() + 1));
    }
    pub(crate) fn listed() {
        LISTED.with(|count| count.set(count.get() + 1));
    }
    /// 交回上次取走之后读了几个、枚举了几项，清零。
    pub(crate) fn take() -> (u64, u64) {
        (
            READS.with(|count| count.replace(0)),
            LISTED.with(|count| count.replace(0)),
        )
    }
}

pub(crate) fn read(dir: &Dir, path: &str, max: usize) -> Result<Option<Vec<u8>>, BrokerError> {
    Ok(read_identified(dir, path, max)?.map(|v| v.0))
}

pub(crate) fn read_identified(
    dir: &Dir,
    path: &str,
    max: usize,
) -> Result<Option<(Vec<u8>, Identity)>, BrokerError> {
    #[cfg(test)]
    probe::read();
    let logical = VaultPath::parse(path).map_err(|_| BrokerError::CorruptWriterState)?;
    let (parent, name) = parent(dir, path)?;
    let parent_identity = Identity::directory(&parent)?;
    let file_path = VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?;
    let file = match FsVaultBroker::open_rooted_file(&parent, &file_path) {
        Ok(file) => file,
        Err(BrokerError::NotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let before = file
        .metadata()
        .map_err(|e| BrokerError::io("stat writer record", e))?;
    FsVaultBroker::validate_file_metadata(&logical, &before)?;
    if before.len() > max as u64 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut bytes = Vec::new();
    (&file)
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| BrokerError::io("read writer record", e))?;
    if bytes.len() > max {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let after = file
        .metadata()
        .map_err(|e| BrokerError::io("restat writer record", e))?;
    let current_parent = parent_dir(dir, path)?;
    if Identity::directory(&current_parent)? != parent_identity {
        return Err(BrokerError::CorruptWriterState);
    }
    let current = FsVaultBroker::open_rooted_file(&current_parent, &file_path)?;
    let current = current
        .metadata()
        .map_err(|e| BrokerError::io("verify writer record", e))?;
    FsVaultBroker::validate_file_metadata(&logical, &current)?;
    if Identity::file(&before) != Identity::file(&current)
        || before.len() != after.len()
        || after.len() != current.len()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || after.ctime() != current.ctime()
        || after.ctime_nsec() != current.ctime_nsec()
        || bytes.len() as u64 != current.len()
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(Some((bytes, Identity::file(&current))))
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, BrokerError> {
    serde_json::from_slice(bytes).map_err(|_| BrokerError::CorruptWriterState)
}

pub(crate) fn save<T: Serialize>(
    dir: &Dir,
    path: &str,
    value: &T,
    check: impl FnOnce() -> Result<(), BrokerError>,
) -> Result<(), BrokerError> {
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > RECORD_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    read(dir, path, RECORD_LIMIT)?;
    let (parent, name) = parent(dir, path)?;
    let identity = Identity::directory(&parent)?;
    FsVaultBroker::atomic_write_dir_checked(&parent, Path::new(name), &bytes, false, || {
        if Identity::directory(&parent_dir(dir, path)?)? != identity {
            return Err(BrokerError::CorruptWriterState);
        }
        check()
    })
}

pub(crate) fn create<T: Serialize>(
    dir: &Dir,
    path: &str,
    value: &T,
    check: impl FnOnce() -> Result<(), BrokerError>,
) -> Result<(), BrokerError> {
    let bytes = serde_json::to_vec(value).map_err(|_| BrokerError::CorruptWriterState)?;
    if bytes.len() > RECORD_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    FsVaultBroker::atomic_write_dir_checked(dir, Path::new(path), &bytes, true, check)
}

fn parent<'a>(dir: &Dir, path: &'a str) -> Result<(Dir, &'a str), BrokerError> {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(BrokerError::CorruptWriterState)?;
    Ok((parent_dir(dir, path)?, name))
}

fn parent_dir(dir: &Dir, path: &str) -> Result<Dir, BrokerError> {
    let parent = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    super::directory(dir, parent.to_str().ok_or(BrokerError::CorruptWriterState)?)
}

pub(crate) fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
