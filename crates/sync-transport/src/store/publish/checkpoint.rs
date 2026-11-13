//! 固定双文件副本；保留源原租约，目标必须为空且与源不存在路径或物理祖先重叠。
use super::io::{directory, probe, regular, same, storage_error, walk};
use super::{File, FilePublisher, Path, Result, StatePublisher, TransportError, paths};
use rustix::fs::{Dir, Mode, OFlags, openat};
use std::{io::Write, os::unix::fs::MetadataExt as _, time::Instant};

pub(super) fn export(
    source: &FilePublisher,
    path: &Path,
    expected: &[u8],
    deadline: Instant,
) -> Result<()> {
    check_deadline(deadline)?;
    source.check()?;
    if source.read()? != expected
        || super::probe(&source.root, paths::STATE_PUBLISH_TEMP, source.owner.0)?.is_some()
    {
        return Err(TransportError::CorruptStore);
    }
    if !paths::private_root_acceptable(path)
        || path.starts_with(&source.root_path)
        || source.root_path.starts_with(path)
    {
        return Err(TransportError::UnsafeStorage);
    }
    let target = walk(path)?;
    directory(&target, source.owner.0)?;
    if ancestor(path, &source.root)? || ancestor(&source.root_path, &target)? {
        return Err(TransportError::UnsafeStorage);
    }
    for entry in Dir::read_from(&target).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Err(TransportError::AlreadyExists);
        }
    }
    let space = rustix::fs::fstatvfs(&target).map_err(storage_error)?;
    if space
        .f_bavail
        .checked_mul(space.f_frsize)
        .is_none_or(|value| value < 2 * 1024 * 1024 * 1024 + paths::STATE_MAX_BYTES)
    {
        return Err(TransportError::Budget);
    }
    create(&target, paths::STATE, expected, source.owner.0)?;
    create(&target, paths::TRANSPORT_LOCK, &[], source.owner.0)?;
    target.sync_all().map_err(|_| TransportError::Unavailable)?;
    let copied = FilePublisher::open_existing(path)?;
    same(&copied.root, &target)?;
    if copied.read()? != expected {
        return Err(TransportError::CorruptStore);
    }
    crate::owned_run::state::decode(expected)?;
    source.check()?;
    if source.read()? != expected {
        return Err(TransportError::CorruptStore);
    }
    same(&walk(path)?, &target)?;
    directory(&target, source.owner.0)?;
    check_deadline(deadline)
}
fn create(root: &File, name: &str, bytes: &[u8], owner: u32) -> Result<()> {
    if !matches!(name, paths::STATE | paths::TRANSPORT_LOCK) {
        return Err(TransportError::UnsafeStorage);
    }
    let mut file = File::from(
        openat(
            root,
            name,
            OFlags::WRONLY
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(storage_error)?,
    );
    file.write_all(bytes)
        .map_err(|_| TransportError::Unavailable)?;
    file.sync_all().map_err(|_| TransportError::Unavailable)?;
    regular(
        &file,
        owner,
        paths::max_bytes(name).ok_or(TransportError::UnsafeStorage)?,
    )?;
    let reopened = probe(root, name, owner)?.ok_or(TransportError::MissingStore)?;
    same(&file, &reopened)
}
fn ancestor(path: &Path, other: &File) -> Result<bool> {
    let expected = other
        .metadata()
        .map_err(|_| TransportError::UnsafeStorage)?;
    for path in path.ancestors() {
        let node = walk(path)?
            .metadata()
            .map_err(|_| TransportError::UnsafeStorage)?;
        if (node.dev(), node.ino()) == (expected.dev(), expected.ino()) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(TransportError::Budget)
    } else {
        Ok(())
    }
}
