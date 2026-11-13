//! 只创建私有独立副本；不使用硬链接、覆盖或重命名现有对象。
use crate::{BrokerError, FsVaultBroker, writer::directory};
use cap_std::fs::Dir;
use rustix::fs::{Mode, OFlags, mkdirat, openat};
use std::{fs::File, path::Path};
const RESERVED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn capacity(parent: &Dir, incoming: u64) -> Result<(), BrokerError> {
    let space =
        rustix::fs::fstatvfs(parent).map_err(|e| BrokerError::io("核对封存可用空间", e.into()))?;
    let needed = RESERVED_BYTES
        .checked_add(incoming)
        .ok_or(BrokerError::WriterLimitExceeded)?;
    if space
        .f_bavail
        .checked_mul(space.f_frsize)
        .is_none_or(|available| available < needed)
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
pub(super) fn dir(parent: &Dir, name: &str) -> Result<Dir, BrokerError> {
    capacity(parent, 4096)?;
    mkdirat(parent, name, Mode::RWXU).map_err(|e| BrokerError::io("创建独立封存目录", e.into()))?;
    FsVaultBroker::sync_directory(parent, Path::new("."))?;
    directory(parent, name)
}
pub(super) fn file(parent: &Dir, name: &str, size: u64) -> Result<File, BrokerError> {
    capacity(parent, size)?;
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|e| BrokerError::io("创建独立封存文件", e.into()))?;
    Ok(File::from(fd))
}
