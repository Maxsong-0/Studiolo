//! 本 crate 唯一做真实 IO 的文件：固定 `transport.lock`、`state.json`
//! 与唯一已登记的发布临时文件 `state.json.publish`。
//!
//! 全部普通单链接、0600 私有权限、真实 UID 核验、目录 FD 相对读写，
//! 文件与父目录 fsync；采用完整原子替换。不对抗可信父目录/同 UID 管理员的恶意更名。

use super::StatePublisher;
use super::paths;
use crate::owned_run::{Result, TransportError};
use std::{
    fs::{File, TryLockError},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// 只在真正持锁后构造；后续构造失败也立即显式释放这把锁。
struct HeldLease(File);

impl HeldLease {
    fn acquire(file: File) -> Result<Self> {
        file.try_lock().map_err(lock_error)?;
        Ok(Self(file))
    }
}

impl Drop for HeldLease {
    fn drop(&mut self) {
        // 副本/继承 FD 可能仍存活；无法在 Drop 报告错误，也不伪造已成功释放。
        let _ = self.0.unlock();
    }
}

fn lock_error(error: TryLockError) -> TransportError {
    match error {
        TryLockError::WouldBlock => TransportError::Busy,
        TryLockError::Error(_) => TransportError::Unavailable,
    }
}

/// 仅测试使用的故障注入；产品路径不经过这些分支。
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishFault {
    /// 只写出一半字节就中止，模拟短写。
    ShortWrite,
    /// 在 fsync 之前失败，模拟持久化失败。
    SyncFailure,
}

/// 固定私有根上的权威状态发布器；持有排他租约。
pub struct FilePublisher {
    root: File,
    root_path: PathBuf,
    _lease: HeldLease,
    owner: (u32, u32),
}

#[cfg(unix)]
impl FilePublisher {
    /// 只打开既有存储：不创建目录、不补空 state、不认领临时残留。
    ///
    /// # Errors
    /// 固定文件缺失返回 `MissingStore`；残留临时文件返回 `TemporaryResidue`。
    pub fn open_existing(path: &Path) -> Result<Self> {
        let publisher = Self::attach(path, false)?;
        if probe(
            &publisher.root,
            paths::STATE_PUBLISH_TEMP,
            publisher.owner.0,
        )?
        .is_some()
        {
            return Err(TransportError::TemporaryResidue);
        }
        probe(&publisher.root, paths::STATE, publisher.owner.0)?
            .ok_or(TransportError::MissingStore)?;
        Ok(publisher)
    }

    /// 管理员显式初始化：三个固定名字都不存在时才建库，并立即发布完整首条记录。
    ///
    /// # Errors
    /// 任一固定名字已存在返回 `AlreadyExists`。
    pub fn initialize(path: &Path, initial: &[u8]) -> Result<Self> {
        let mut publisher = Self::attach(path, true)?;
        publisher.replace(initial)?;
        Ok(publisher)
    }

    /// 显式丢弃上次发布留下的临时文件；它从来不是授权状态。
    ///
    /// # Errors
    /// 根身份或权限不符返回 `UnsafeStorage`。
    pub fn discard_residue(path: &Path) -> Result<()> {
        use rustix::fs::{AtFlags, unlinkat};
        let publisher = Self::attach(path, false)?;
        let root = &publisher.root;
        if probe(root, paths::STATE_PUBLISH_TEMP, publisher.owner.0)?.is_some() {
            unlinkat(root, paths::STATE_PUBLISH_TEMP, AtFlags::empty()).map_err(storage_error)?;
            root.sync_all().map_err(|_| TransportError::Unavailable)?;
        }
        Ok(())
    }

    fn attach(path: &Path, initialize: bool) -> Result<Self> {
        use rustix::fs::{Mode, OFlags, openat};
        if !paths::private_root_acceptable(path) {
            return Err(TransportError::UnsafeStorage);
        }
        let owner = caller()?;
        let root = walk(path)?;
        directory(&root, owner.0)?;
        if initialize {
            for name in paths::fixed_names() {
                if probe(&root, name, owner.0)?.is_some() {
                    return Err(TransportError::AlreadyExists);
                }
            }
        }
        let flags = OFlags::RDWR
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC
            | if initialize {
                OFlags::CREATE | OFlags::EXCL
            } else {
                OFlags::empty()
            };
        let lease = File::from(
            openat(
                &root,
                paths::TRANSPORT_LOCK,
                flags,
                Mode::from_raw_mode(0o600),
            )
            .map_err(storage_error)?,
        );
        regular(&lease, owner.0, paths::LOCK_MAX_BYTES)?;
        let lease = HeldLease::acquire(lease)?;
        if initialize {
            lease
                .0
                .sync_all()
                .map_err(|_| TransportError::Unavailable)?;
            root.sync_all().map_err(|_| TransportError::Unavailable)?;
        }
        let publisher = Self {
            root,
            root_path: path.to_owned(),
            _lease: lease,
            owner,
        };
        publisher.check()?;
        Ok(publisher)
    }

    fn publish(&mut self, bytes: &[u8], #[cfg(test)] fault: Option<PublishFault>) -> Result<()> {
        use rustix::fs::{Mode, OFlags, openat, renameat};
        if bytes.len() as u64 > paths::STATE_MAX_BYTES || bytes.is_empty() {
            return Err(TransportError::Budget);
        }
        self.check()?;
        if probe(&self.root, paths::STATE_PUBLISH_TEMP, self.owner.0)?.is_some() {
            return Err(TransportError::TemporaryResidue);
        }
        let mut temp = File::from(
            openat(
                &self.root,
                paths::STATE_PUBLISH_TEMP,
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(storage_error)?,
        );
        #[cfg(test)]
        let payload = match fault {
            Some(PublishFault::ShortWrite) => &bytes[..bytes.len() / 2],
            _ => bytes,
        };
        #[cfg(not(test))]
        let payload = bytes;
        temp.write_all(payload)
            .map_err(|_| TransportError::Unavailable)?;
        #[cfg(test)]
        if fault.is_some() {
            // 短写与 fsync 失败都在这里中止：临时文件留在原地，权威记录保持旧版本。
            return Err(TransportError::Unavailable);
        }
        temp.sync_all().map_err(|_| TransportError::Unavailable)?;
        regular(&temp, self.owner.0, paths::STATE_MAX_BYTES)?;
        drop(temp);
        self.check()?;
        renameat(
            &self.root,
            paths::STATE_PUBLISH_TEMP,
            &self.root,
            paths::STATE,
        )
        .map_err(storage_error)?;
        self.root
            .sync_all()
            .map_err(|_| TransportError::Unavailable)?;
        self.check()
    }

    /// 仅测试：带故障注入的发布。
    ///
    /// # Errors
    /// 注入故障时返回 `Unavailable`，且权威记录必须保持旧版本。
    #[cfg(test)]
    pub fn replace_faulted(&mut self, bytes: &[u8], fault: PublishFault) -> Result<()> {
        self.publish(bytes, Some(fault))
    }
}

#[cfg(unix)]
impl super::StatePublisher for FilePublisher {
    fn read(&self) -> Result<Vec<u8>> {
        self.check()?;
        let file =
            probe(&self.root, paths::STATE, self.owner.0)?.ok_or(TransportError::MissingStore)?;
        let length = file
            .metadata()
            .map_err(|_| TransportError::UnsafeStorage)?
            .len();
        let mut bytes = Vec::new();
        (&file)
            .take(paths::STATE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| TransportError::Unavailable)?;
        if bytes.len() as u64 != length || bytes.is_empty() {
            return Err(TransportError::CorruptStore);
        }
        self.check()?;
        Ok(bytes)
    }

    fn replace(&mut self, bytes: &[u8]) -> Result<()> {
        self.publish(
            bytes,
            #[cfg(test)]
            None,
        )
    }

    fn check(&self) -> Result<()> {
        if caller()? != self.owner {
            return Err(TransportError::UnsafeStorage);
        }
        // 先查持有的根 FD 本身还在不在，再查这条路径现在指向的是不是同一个对象。
        directory(&self.root, self.owner.0)?;
        let current = walk(&self.root_path)?;
        directory(&current, self.owner.0)?;
        same(&current, &self.root)?;
        regular(&self._lease.0, self.owner.0, paths::LOCK_MAX_BYTES)?;
        let lease = probe(&self.root, paths::TRANSPORT_LOCK, self.owner.0)?
            .ok_or(TransportError::MissingStore)?;
        same(&lease, &self._lease.0)
    }

    fn owner_identity(&self) -> (u32, u32) {
        self.owner
    }
}

#[cfg(unix)]
fn caller() -> Result<(u32, u32)> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(TransportError::UnsafeStorage);
    }
    Ok((geteuid().as_raw(), getegid().as_raw()))
}

#[cfg(unix)]
fn walk(path: &Path) -> Result<File> {
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
fn directory(file: &File, owner: u32) -> Result<()> {
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
fn regular(file: &File, owner: u32, max: u64) -> Result<()> {
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
fn probe(root: &File, name: &str, owner: u32) -> Result<Option<File>> {
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
fn same(left: &File, right: &File) -> Result<()> {
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
fn storage_error(error: rustix::io::Errno) -> TransportError {
    if error == rustix::io::Errno::NOENT {
        TransportError::MissingStore
    } else {
        TransportError::UnsafeStorage
    }
}
