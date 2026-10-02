//! 固定私有根/DB/lease；NOFOLLOW/NONBLOCK 防链接与 FIFO 阻塞。
//! 不对抗可信父目录/同 UID 管理员恶意 ABA 更名；不导出任何文件能力。

use super::model::{BridgeError, Result};
use std::{
    fs::{File, TryLockError},
    path::{Path, PathBuf},
};
pub(super) const DB_MAX: u64 = 256 * 1024 * 1024;
pub(super) const WAL_MAX: u64 = (4096 + 24) * 65536 + 32;
// 只在真正持锁后构造；后续 Storage 构造失败也立即显式释放这把锁。
struct HeldLease(File);
impl HeldLease {
    fn acquire(file: File) -> Result<Self> {
        file.try_lock().map_err(lock_error)?;
        Ok(Self(file))
    }
}
impl Drop for HeldLease {
    fn drop(&mut self) {
        // 副本/继承 FD 可能仍存活；不能只依靠最后一个 FD 关闭。
        // 无法报告 Drop 错误，仍关闭本 FD；不重试或伪造已成功释放。
        let _ = self.0.unlock();
    }
}
pub(super) fn lock_error(error: TryLockError) -> BridgeError {
    match error {
        TryLockError::WouldBlock => BridgeError::Busy,
        TryLockError::Error(_) => BridgeError::Unavailable,
    }
}
#[cfg(test)]
type AcquireHook<'a> = &'a mut dyn FnMut(&File) -> Result<()>;
pub(super) struct Storage {
    pub database: PathBuf,
    root_path: PathBuf,
    root: File,
    database_pin: File,
    _lease: HeldLease,
    #[cfg(unix)]
    owner: u32,
}
#[cfg(unix)]
impl Storage {
    pub fn open(path: &Path, initialize: bool) -> Result<Self> {
        Self::open_inner(
            path,
            initialize,
            #[cfg(test)]
            None,
        )
    }
    pub(super) fn open_inner(
        path: &Path,
        initialize: bool,
        #[cfg(test)] hook: Option<AcquireHook<'_>>,
    ) -> Result<Self> {
        use rustix::fs::{Mode, OFlags, openat};
        let owner = caller()?;
        let root = walk(path)?;
        directory(&root, owner)?;
        if initialize {
            for name in [
                "bridge.lock",
                "bridge.sqlite3",
                "bridge.sqlite3-wal",
                "bridge.sqlite3-shm",
                "bridge.sqlite3-journal",
            ] {
                if inspect(&root, name, owner, true)?.is_some() {
                    return Err(BridgeError::AlreadyExists);
                }
            }
        } else {
            inspect(&root, "bridge.lock", owner, false)?;
            inspect(&root, "bridge.sqlite3", owner, false)?;
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
            openat(&root, "bridge.lock", flags, Mode::from_raw_mode(0o600))
                .map_err(storage_error)?,
        );
        regular(&lease, owner, 4096)?;
        let lease = HeldLease::acquire(lease)?;
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(&lease.0)?;
        }
        let database_pin = File::from(
            openat(&root, "bridge.sqlite3", flags, Mode::from_raw_mode(0o600))
                .map_err(storage_error)?,
        );
        regular(&database_pin, owner, DB_MAX)?;
        let storage = Self {
            database: path.join("bridge.sqlite3"),
            root_path: path.to_owned(),
            root,
            database_pin,
            _lease: lease,
            owner,
        };
        storage.check()?;
        if initialize {
            storage
                ._lease
                .0
                .sync_all()
                .map_err(|_| BridgeError::Unavailable)?;
            storage.check()?;
            storage
                .database_pin
                .sync_all()
                .map_err(|_| BridgeError::Unavailable)?;
            storage.check()?;
            storage
                .root
                .sync_all()
                .map_err(|_| BridgeError::Unavailable)?;
            storage.check()?;
        } else if storage
            .database_pin
            .metadata()
            .map_err(|_| BridgeError::UnsafeStorage)?
            .len()
            == 0
        {
            return Err(BridgeError::CorruptStore);
        }
        Ok(storage)
    }
    pub fn check(&self) -> Result<()> {
        if caller()? != self.owner {
            return Err(BridgeError::UnsafeStorage);
        }
        let current = walk(&self.root_path)?;
        directory(&current, self.owner)?;
        same(&current, &self.root)?;
        regular(&self.database_pin, self.owner, DB_MAX)?;
        regular(&self._lease.0, self.owner, 4096)?;
        for (name, pin) in [
            ("bridge.sqlite3", &self.database_pin),
            ("bridge.lock", &self._lease.0),
        ] {
            let stat =
                inspect(&self.root, name, self.owner, false)?.ok_or(BridgeError::MissingStore)?;
            same_stat(&stat, pin)?;
        }
        for name in [
            "bridge.sqlite3-wal",
            "bridge.sqlite3-shm",
            "bridge.sqlite3-journal",
        ] {
            inspect(&self.root, name, self.owner, true)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn duplicate_lease_for_test(&self) -> Result<File> {
        self._lease
            .0
            .try_clone()
            .map_err(|_| BridgeError::Unavailable)
    }
}
#[cfg(unix)]
fn caller() -> Result<u32> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(geteuid().as_raw())
}
#[cfg(unix)]
fn walk(path: &Path) -> Result<File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    use std::path::Component;
    if !path.is_absolute() {
        return Err(BridgeError::UnsafeStorage);
    }
    let flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut fd = File::from(open("/", flags, Mode::empty()).map_err(storage_error)?);
    for c in path.components() {
        match c {
            Component::RootDir => {}
            Component::Normal(n) => {
                fd = File::from(openat(&fd, n, flags, Mode::empty()).map_err(storage_error)?)
            }
            _ => return Err(BridgeError::UnsafeStorage),
        }
    }
    Ok(fd)
}
#[cfg(unix)]
fn directory(f: &File, owner: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = f.metadata().map_err(|_| BridgeError::UnsafeStorage)?;
    if !m.is_dir() || m.uid() != owner || m.mode() & 0o7777 != 0o700 {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(())
}
#[cfg(unix)]
fn regular(f: &File, owner: u32, max: u64) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = f.metadata().map_err(|_| BridgeError::UnsafeStorage)?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != owner
        || m.mode() & 0o7777 != 0o600
        || m.len() > max
    {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(())
}
#[cfg(unix)]
// 不能为检查临时打开再关闭 DB/SHM：POSIX close 会释放本进程 SQLite 的锁。
// 只对已钉住的目录作不跟随链接的元数据查询，pin 生命周期保持不变。
fn inspect(
    root: &File,
    name: &str,
    owner: u32,
    optional: bool,
) -> Result<Option<rustix::fs::Stat>> {
    use rustix::fs::{AtFlags, FileType, statat};
    let stat = match statat(root, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) if optional => return Ok(None),
        Err(e) => return Err(storage_error(e)),
    };
    let max = match name {
        "bridge.lock" => 4096,
        "bridge.sqlite3-wal" => WAL_MAX,
        "bridge.sqlite3-shm" => 4 * 1024 * 1024,
        _ => DB_MAX,
    };
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_nlink != 1
        || stat.st_uid != owner
        || stat.st_mode & 0o7777 != 0o600
        || u64::try_from(stat.st_size).map_or(true, |size| size > max)
    {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(Some(stat))
}
#[cfg(unix)]
fn same_stat(current: &rustix::fs::Stat, pin: &File) -> Result<()> {
    let held = rustix::fs::fstat(pin).map_err(|_| BridgeError::UnsafeStorage)?;
    if (current.st_dev, current.st_ino) != (held.st_dev, held.st_ino) {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(())
}
#[cfg(unix)]
fn same(a: &File, b: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let a = a.metadata().map_err(|_| BridgeError::UnsafeStorage)?;
    let b = b.metadata().map_err(|_| BridgeError::UnsafeStorage)?;
    if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
        return Err(BridgeError::UnsafeStorage);
    }
    Ok(())
}
#[cfg(unix)]
fn storage_error(e: rustix::io::Errno) -> BridgeError {
    if e == rustix::io::Errno::NOENT {
        BridgeError::MissingStore
    } else {
        BridgeError::UnsafeStorage
    }
}
#[cfg(not(unix))]
impl Storage {
    pub fn open(_: &Path, _: bool) -> Result<Self> {
        Err(BridgeError::UnsupportedPlatform)
    }
    pub fn check(&self) -> Result<()> {
        Err(BridgeError::UnsupportedPlatform)
    }
}
