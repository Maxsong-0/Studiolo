//! 固定状态目录的只读路径/权限校验，不授予文件创建能力。
use crate::store::model::StoreError;
use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
use std::path::{Component, Path};

pub(super) fn caller() -> Result<u32, StoreError> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(StoreError::UnsafeStorage);
    }
    Ok(geteuid().as_raw())
}

/// 从 `/` 逐段打开；任何一段是链接或非目录都失败关闭。
pub(super) fn walk(path: &Path) -> Result<rustix::fd::OwnedFd, StoreError> {
    if !path.is_absolute() {
        return Err(StoreError::UnsafeStorage);
    }
    let flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(|_| StoreError::UnsafeStorage)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = openat(&directory, name, flags, Mode::empty()).map_err(|error| {
                    if error == rustix::io::Errno::NOENT {
                        StoreError::MissingStore
                    } else {
                        StoreError::UnsafeStorage
                    }
                })?;
            }
            _ => return Err(StoreError::UnsafeStorage),
        }
    }
    Ok(directory)
}

pub(super) fn check_directory(
    directory: &rustix::fd::OwnedFd,
    owner: u32,
) -> Result<(), StoreError> {
    let stat = fstat(directory).map_err(|_| StoreError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != owner
        || Mode::from_raw_mode(stat.st_mode) != Mode::RWXU
    {
        return Err(StoreError::UnsafeStorage);
    }
    Ok(())
}

pub(super) fn check_regular(
    file: &rustix::fd::OwnedFd,
    owner: u32,
    max: u64,
) -> Result<u64, StoreError> {
    let stat = fstat(file).map_err(|_| StoreError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != owner
        || stat.st_nlink != 1
        || Mode::from_raw_mode(stat.st_mode) != Mode::RUSR | Mode::WUSR
        || stat.st_size < 0
    {
        return Err(StoreError::UnsafeStorage);
    }
    let length = stat.st_size.unsigned_abs();
    if length > max {
        return Err(StoreError::LimitExceeded);
    }
    Ok(length)
}
