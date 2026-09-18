//! 新目录恢复自己的根打开与独占租约。
//!
//! 与 `writer::lease` 同样从 `/` 逐段 NOFOLLOW 打开，但不创建任何目录：
//! 恢复只重绑定已经存在的新目录，不替调用者造一份新存储。
//! 这里独立取得排他 flock，第二写者仍在运行时立刻失败，不会边改边被写。

use super::RestoreError;
use crate::BrokerError;
use cap_std::fs::Dir;
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open, openat};
use std::{
    fs::File,
    path::{Component, Path, PathBuf},
};

/// 只在成功取得排他锁后构造；不允许复制所有权。
pub(super) struct RestoreLease(File);

impl Drop for RestoreLease {
    fn drop(&mut self) {
        // Drop 不能报告错误；仍关闭自身 FD，不重试也不伪造释放成功。
        let _ = flock(&self.0, FlockOperation::Unlock);
    }
}

/// 规范化为绝对路径；`..` 一律拒绝。
pub(super) fn absolute(path: &Path) -> Result<PathBuf, RestoreError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(RestoreError::Broker(BrokerError::WriterBindingMismatch));
    }
    Ok(path.components().collect())
}

/// 从 `/` 逐段打开目录；不跟随任何祖先链接，也不创建缺失的目录。
pub(super) fn walk(path: &Path) -> Result<Dir, RestoreError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = open("/", flags, Mode::empty())
        .map_err(|e| RestoreError::Broker(BrokerError::io("open restore anchor", e.into())))?;
    let mut dir = Dir::from_std_file(File::from(fd));
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let fd = openat(&dir, name, flags, Mode::empty()).map_err(|e| {
            if e == rustix::io::Errno::NOENT {
                RestoreError::NewRootIncomplete
            } else {
                RestoreError::Broker(BrokerError::io("open restore directory", e.into()))
            }
        })?;
        dir = Dir::from_std_file(File::from(fd));
    }
    Ok(dir)
}

/// 取得新 canonical 根的排他租约；已有写者即 `NewRootBusy`。
pub(super) fn acquire(root: &Dir) -> Result<RestoreLease, RestoreError> {
    let file = root
        .try_clone()
        .map_err(|e| RestoreError::Broker(BrokerError::io("retain restore lease", e)))?
        .into_std_file();
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
        if e == rustix::io::Errno::WOULDBLOCK {
            RestoreError::NewRootBusy
        } else {
            RestoreError::Broker(BrokerError::io("acquire restore lease", e.into()))
        }
    })?;
    Ok(RestoreLease(file))
}

/// 两个根不得互相包含。
pub(super) fn disjoint(vault: &Path, data: &Path) -> Result<(), RestoreError> {
    if vault.starts_with(data) || data.starts_with(vault) {
        return Err(RestoreError::Broker(BrokerError::WriterBindingMismatch));
    }
    Ok(())
}
