//! 原目录与固定空锁文件共同固定一个写者；不依据 PID 文本判断或删除 stale 锁。
use super::{
    StoreError,
    paths::{caller, check_directory, check_regular, walk},
};
use rustix::{
    fd::OwnedFd,
    fs::{FlockOperation, Mode, OFlags, flock, fstat, fsync, openat},
};
use std::path::{Path, PathBuf};
pub(super) struct Lease {
    lock: OwnedFd,
    path: PathBuf,
    root: (u64, u64),
}
const LOCK: &str = "backup.lock";
impl Lease {
    pub fn acquire(path: &Path, root: &OwnedFd, initialize: bool) -> Result<Self, StoreError> {
        let owner = caller()?;
        let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        let lock = if initialize {
            match openat(
                root,
                LOCK,
                flags | OFlags::CREATE | OFlags::EXCL,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => {
                    fsync(&fd).map_err(|_| StoreError::Unavailable)?;
                    fsync(root).map_err(|_| StoreError::Unavailable)?;
                    fd
                }
                Err(rustix::io::Errno::EXIST) => return Err(StoreError::AlreadyExists),
                Err(e) => return Err(error(e)),
            }
        } else {
            openat(root, LOCK, flags, Mode::empty()).map_err(error)?
        };
        if check_regular(&lock, owner, 0)? != 0 {
            return Err(StoreError::UnsafeStorage);
        }
        flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            if e == rustix::io::Errno::AGAIN {
                StoreError::Busy
            } else {
                StoreError::UnsafeStorage
            }
        })?;
        let lease = Self {
            lock,
            path: path.to_owned(),
            root: identity(root)?,
        };
        lease.check(root)?;
        Ok(lease)
    }
    pub fn check(&self, root: &OwnedFd) -> Result<(), StoreError> {
        let owner = caller()?;
        let current = walk(&self.path).map_err(|_| StoreError::UnsafeStorage)?;
        check_directory(&current, owner)?;
        if identity(&current)? != self.root || identity(root)? != self.root {
            return Err(StoreError::UnsafeStorage);
        }
        let current_lock = openat(
            root,
            LOCK,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(error)?;
        check_regular(&self.lock, owner, 0)?;
        check_regular(&current_lock, owner, 0)?;
        if identity(&current_lock)? != identity(&self.lock)? {
            return Err(StoreError::UnsafeStorage);
        }
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = flock(&self.lock, FlockOperation::Unlock);
    }
}
fn identity(fd: &OwnedFd) -> Result<(u64, u64), StoreError> {
    let stat = fstat(fd).map_err(|_| StoreError::UnsafeStorage)?;
    Ok((stat.st_dev, stat.st_ino))
}
fn error(error: rustix::io::Errno) -> StoreError {
    if error == rustix::io::Errno::NOENT {
        StoreError::MissingStore
    } else {
        StoreError::UnsafeStorage
    }
}
