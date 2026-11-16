//! 固定发布器的 nofollow、权限与身份核对；不对外暴露文件能力。
use super::{File, Path, Result, TransportError, paths};
#[cfg(unix)]
pub(super) fn caller() -> Result<(u32, u32)> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(TransportError::UnsafeStorage);
    }
    Ok((geteuid().as_raw(), getegid().as_raw()))
}

#[cfg(unix)]
pub(super) fn walk(path: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    use std::path::Component;
    let flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut fd = File::from(open("/", flags, Mode::empty()).map_err(storage_error)?);
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                fd = File::from(openat(&fd, name, flags, Mode::empty()).map_err(storage_error)?);
            }
            _ => return Err(TransportError::UnsafeStorage),
        }
    }
    Ok(fd)
}

#[cfg(unix)]
pub(super) fn directory(file: &File, owner: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| TransportError::UnsafeStorage)?;
    // 链接数归零＝这个目录已被删掉。对持有的根 FD 检查它，是唯一能穿透
    // 「inode 号被同名新目录原样复用」的信号；只比 (dev, ino) 在 Linux 上看不出替换。
    if !metadata.is_dir()
        || metadata.nlink() == 0
        || metadata.uid() != owner
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(TransportError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn regular(file: &File, owner: u32, max: u64) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| TransportError::UnsafeStorage)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != owner
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > max
    {
        return Err(TransportError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn probe(root: &File, name: &str, owner: u32) -> Result<Option<File>> {
    use rustix::fs::{Mode, OFlags, openat};
    let max = paths::max_bytes(name).ok_or(TransportError::UnsafeStorage)?;
    let file = match openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(storage_error(error)),
    };
    regular(&file, owner, max)?;
    Ok(Some(file))
}

#[cfg(unix)]
pub(super) fn same(left: &File, right: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata().map_err(|_| TransportError::UnsafeStorage)?;
    let right = right
        .metadata()
        .map_err(|_| TransportError::UnsafeStorage)?;
    if (left.dev(), left.ino()) != (right.dev(), right.ino()) {
        return Err(TransportError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn storage_error(error: rustix::io::Errno) -> TransportError {
    if error == rustix::io::Errno::NOENT {
        TransportError::MissingStore
    } else {
        TransportError::UnsafeStorage
    }
}
