//! 封存专用的有界 FD 助手；不删除、不覆盖，只创建固定状态名或已验证的内容地址。
use super::{
    INDEX_FILE, OBJECT_MAX_BYTES, PlaneError, caller, create_at, entry_names, flush,
    open_directory_path, open_regular_at, private_directory, private_regular, read_fully,
    write_fully,
};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{Stat, fstat, fstatvfs};
use std::path::Path;
use std::time::Instant;

pub(in crate::sync_plane) fn deadline(until: Instant) -> Result<(), PlaneError> {
    if Instant::now() >= until {
        Err(PlaneError::Storage)
    } else {
        Ok(())
    }
}
pub(in crate::sync_plane) fn metadata(fd: BorrowedFd<'_>) -> Result<Stat, PlaneError> {
    fstat(fd).map_err(|_| PlaneError::Storage)
}
pub(in crate::sync_plane) fn same(a: &Stat, b: &Stat) -> Result<(), PlaneError> {
    if (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino) {
        return Err(PlaneError::Storage);
    }
    Ok(())
}
pub(in crate::sync_plane) fn unchanged(a: &Stat, b: &Stat) -> Result<(), PlaneError> {
    same(a, b)?;
    if (
        a.st_size,
        a.st_mtime,
        a.st_mtime_nsec,
        a.st_ctime,
        a.st_ctime_nsec,
        a.st_mode,
        a.st_nlink,
        a.st_uid,
    ) != (
        b.st_size,
        b.st_mtime,
        b.st_mtime_nsec,
        b.st_ctime,
        b.st_ctime_nsec,
        b.st_mode,
        b.st_nlink,
        b.st_uid,
    ) {
        return Err(PlaneError::Storage);
    }
    Ok(())
}
pub(in crate::sync_plane) fn root(
    path: &Path,
    expected: BorrowedFd<'_>,
    uid: u32,
) -> Result<(), PlaneError> {
    let (_, stat) = open_directory_path(path).map_err(|_| PlaneError::Storage)?;
    if !private_directory(&stat, uid) {
        return Err(PlaneError::Storage);
    }
    same(&stat, &metadata(expected)?)
}
pub(in crate::sync_plane) fn target(
    path: &Path,
    source: &Path,
    uid: u32,
    bytes: u64,
) -> Result<OwnedFd, PlaneError> {
    if path.starts_with(source) || source.starts_with(path) {
        return Err(PlaneError::Storage);
    }
    let (target, stat) = open_directory_path(path).map_err(|_| PlaneError::Storage)?;
    let (_, source_stat) = open_directory_path(source).map_err(|_| PlaneError::Storage)?;
    if !private_directory(&stat, uid)
        || !entry_names(target.as_fd())
            .map_err(|_| PlaneError::Storage)?
            .is_empty()
    {
        return Err(PlaneError::Storage);
    }
    for (path, other) in [(path, source_stat), (source, stat)] {
        for ancestor in path.ancestors() {
            let (_, current) = open_directory_path(ancestor).map_err(|_| PlaneError::Storage)?;
            if (current.st_dev, current.st_ino) == (other.st_dev, other.st_ino) {
                return Err(PlaneError::Storage);
            }
        }
    }
    let space = fstatvfs(&target).map_err(|_| PlaneError::Storage)?;
    if space
        .f_bavail
        .checked_mul(space.f_frsize)
        .is_none_or(|free| free < bytes.saturating_add(2 * 1024 * 1024 * 1024))
    {
        return Err(PlaneError::Storage);
    }
    Ok(target)
}
pub(in crate::sync_plane) fn read(
    root: BorrowedFd<'_>,
    name: &str,
    uid: u32,
    max: usize,
) -> Result<Option<(Vec<u8>, Stat)>, PlaneError> {
    let Some((file, length)) =
        open_regular_at(root, name, uid, max).map_err(|_| PlaneError::Storage)?
    else {
        return Ok(None);
    };
    let before = metadata(file.as_fd())?;
    let bytes = read_fully(file.as_fd(), length).map_err(|_| PlaneError::Storage)?;
    let (reopened, _) = open_regular_at(root, name, uid, max)
        .map_err(|_| PlaneError::Storage)?
        .ok_or(PlaneError::Storage)?;
    unchanged(&before, &metadata(file.as_fd())?)?;
    unchanged(&before, &metadata(reopened.as_fd())?)?;
    Ok(Some((bytes, before)))
}
pub(in crate::sync_plane) fn digest_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(in crate::sync_plane) fn create(
    root: BorrowedFd<'_>,
    name: &str,
) -> Result<OwnedFd, PlaneError> {
    if !matches!(name, INDEX_FILE | "plane-state.json" | "plane.lock") && !digest_name(name) {
        return Err(PlaneError::Storage);
    }
    let (file, stat) = create_at(root, name, true).map_err(|_| PlaneError::Storage)?;
    if private_regular(&stat, caller()?, OBJECT_MAX_BYTES).is_none() {
        return Err(PlaneError::Storage);
    }
    Ok(file)
}
pub(in crate::sync_plane) fn write(
    root: BorrowedFd<'_>,
    name: &str,
    bytes: &[u8],
    uid: u32,
) -> Result<(), PlaneError> {
    let file = create(root, name)?;
    write_fully(file.as_fd(), bytes)
        .and_then(|()| flush(file.as_fd()))
        .map_err(|_| PlaneError::Storage)?;
    let (copied, stat) = read(root, name, uid, bytes.len())?.ok_or(PlaneError::Storage)?;
    unchanged(&stat, &metadata(file.as_fd())?)?;
    if copied != bytes {
        return Err(PlaneError::Storage);
    }
    Ok(())
}
pub(in crate::sync_plane) fn sync(fd: BorrowedFd<'_>) -> Result<(), PlaneError> {
    flush(fd).map_err(|_| PlaneError::Storage)
}
