//! 本 crate 唯一做真实 IO 的文件：固定 `transport.lock`、`state.json`
//! 与唯一已登记的发布临时文件 `state.json.publish`。
//!
//! 全部普通单链接、0600 私有权限、真实 UID 核验、目录 FD 相对读写，
//! 文件与父目录 fsync；采用完整原子替换。不对抗可信父目录/同 UID 管理员的恶意更名。

mod checkpoint;
mod io;
use super::StatePublisher;
use super::paths;
use crate::owned_run::{Result, TransportError};
use io::{caller, directory, probe, regular, same, storage_error, walk};
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
#[derive(Clone, Copy, Debug)]
pub enum PublishFault {
    /// 只写出一半字节就报错（盘满）。
    ShortWrite,
    /// 在 fsync 处报错（盘坏）。
    SyncFailure,
    /// 写到一半进程被杀：什么都来不及清。
    KilledMidWrite,
    /// 写全、落了盘、还没改名时进程被杀（断电后改名没落盘也是这个样子）。
    KilledBeforeRename,
    /// 改名成功之后，目录落盘报错。
    DirSyncFailure,
    /// 写完、核对之前，用例拿着私有根去动临时文件那个名字。
    Tampered(fn(&Path)),
}

/// 固定私有根上的权威状态发布器；持有排他租约。
pub struct FilePublisher {
    root: File,
    root_path: PathBuf,
    _lease: HeldLease,
    owner: (u32, u32),
    /// 打开时认下的残留；只握着它的身份，从不读它的内容。
    residue: Option<File>,
}

#[cfg(unix)]
impl FilePublisher {
    /// 只打开既有存储：不创建目录、不补空 state、不认领临时残留。
    ///
    /// crate 外只有这一个打开入口，拿不到带着残留的发布器：
    ///
    /// ```compile_fail
    /// let root = std::path::Path::new("/srv/studiolo/transport");
    /// let _ = studiolo_sync_transport::store::FilePublisher::open_with_residue(root);
    /// ```
    ///
    /// # 错误
    /// 固定文件缺失返回 `MissingStore`；残留临时文件返回 `TemporaryResidue`。
    pub fn open_existing(path: &Path) -> Result<Self> {
        let publisher = Self::attach(path, false)?;
        if publisher.temporary()?.is_some() {
            return Err(TransportError::TemporaryResidue);
        }
        publisher.require_state()?;
        Ok(publisher)
    }

    /// 同 [`Self::open_existing`]，但认下形态合规的残留、不动它；核过权威记录之后才许
    /// [`super::StatePublisher::discard_residue`]。只给 crate 内的 owner 与重绑定用。
    ///
    /// # 错误
    /// 固定文件缺失返回 `MissingStore`；残留不是本人 0600 单链接普通文件返回 `UnsafeStorage`。
    pub(crate) fn open_with_residue(path: &Path) -> Result<Self> {
        let mut publisher = Self::attach(path, false)?;
        publisher.residue = publisher.temporary()?;
        publisher.require_state()?;
        Ok(publisher)
    }

    fn temporary(&self) -> Result<Option<File>> {
        probe(&self.root, paths::STATE_PUBLISH_TEMP, self.owner.0)
    }

    fn require_state(&self) -> Result<()> {
        probe(&self.root, paths::STATE, self.owner.0)?
            .map(|_| ())
            .ok_or(TransportError::MissingStore)
    }

    /// 管理员显式初始化：三个固定名字都不存在时才建库，并立即发布完整首条记录。
    ///
    /// # 错误
    /// 任一固定名字已存在返回 `AlreadyExists`。
    pub fn initialize(path: &Path, initial: &[u8]) -> Result<Self> {
        let mut publisher = Self::attach(path, true)?;
        publisher.replace(initial)?;
        Ok(publisher)
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
            residue: None,
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
        if self.temporary()?.is_some() {
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
        // 改名之前任一步失败，权威记录都还是旧的。
        let installed = self
            .fill(
                &mut temp,
                bytes,
                #[cfg(test)]
                fault,
            )
            .and_then(|()| {
                renameat(
                    &self.root,
                    paths::STATE_PUBLISH_TEMP,
                    &self.root,
                    paths::STATE,
                )
                .map_err(storage_error)
            });
        if let Err(error) = installed {
            // 被杀的进程来不及收拾；报错的要收走自己刚建的，否则之后每次发布都卡在残留上。
            #[cfg(test)]
            if matches!(
                fault,
                Some(PublishFault::KilledMidWrite | PublishFault::KilledBeforeRename)
            ) {
                return Err(error);
            }
            self.remove_own(&temp);
            return Err(error);
        }
        #[cfg(test)]
        if matches!(fault, Some(PublishFault::DirSyncFailure)) {
            return Err(TransportError::Unavailable);
        }
        self.root
            .sync_all()
            .map_err(|_| TransportError::Unavailable)?;
        self.check()
    }

    /// 写全、落盘，再核一遍临时文件、租约和根；成功后才许改名。
    fn fill(
        &self,
        temp: &mut File,
        bytes: &[u8],
        #[cfg(test)] fault: Option<PublishFault>,
    ) -> Result<()> {
        #[cfg(test)]
        let payload = match fault {
            Some(PublishFault::ShortWrite | PublishFault::KilledMidWrite) => {
                &bytes[..bytes.len() / 2]
            }
            _ => bytes,
        };
        #[cfg(not(test))]
        let payload = bytes;
        temp.write_all(payload)
            .map_err(|_| TransportError::Unavailable)?;
        #[cfg(test)]
        match fault {
            None | Some(PublishFault::KilledBeforeRename | PublishFault::DirSyncFailure) => {}
            Some(PublishFault::Tampered(tamper)) => tamper(&self.root_path),
            Some(_) => return Err(TransportError::Unavailable),
        }
        temp.sync_all().map_err(|_| TransportError::Unavailable)?;
        regular(temp, self.owner.0, paths::STATE_MAX_BYTES)?;
        #[cfg(test)]
        if matches!(fault, Some(PublishFault::KilledBeforeRename)) {
            return Err(TransportError::Unavailable);
        }
        self.check()
    }

    /// 收走本次发布自己建的临时文件：只认名字还指着手里这一个对象，不看形态
    /// （报错的原因可能正是它的权限或链接数不对）；收不掉也不盖住原错误。
    fn remove_own(&self, temp: &File) {
        use rustix::fs::{AtFlags, fstat, statat, unlinkat};
        let named = statat(
            &self.root,
            paths::STATE_PUBLISH_TEMP,
            AtFlags::SYMLINK_NOFOLLOW,
        );
        let (Ok(named), Ok(own)) = (named, fstat(temp)) else {
            return;
        };
        if (named.st_dev, named.st_ino) == (own.st_dev, own.st_ino)
            && unlinkat(&self.root, paths::STATE_PUBLISH_TEMP, AtFlags::empty()).is_ok()
        {
            let _ = self.root.sync_all();
        }
    }

    /// 仅测试：带故障注入的发布。
    ///
    /// # 错误
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

    fn has_residue(&self) -> bool {
        self.residue.is_some()
    }

    /// 删之前再核一遍租约、根，和它还是打开时的那一个。
    fn discard_residue(&mut self) -> Result<bool> {
        use rustix::fs::{AtFlags, unlinkat};
        let Some(opened) = &self.residue else {
            return Ok(false);
        };
        self.check()?;
        let current = self.temporary()?.ok_or(TransportError::MissingStore)?;
        same(&current, opened)?;
        unlinkat(&self.root, paths::STATE_PUBLISH_TEMP, AtFlags::empty()).map_err(storage_error)?;
        self.root
            .sync_all()
            .map_err(|_| TransportError::Unavailable)?;
        self.residue = None;
        self.check()?;
        Ok(true)
    }

    fn export_checkpoint(
        &self,
        target: &Path,
        expected: &[u8],
        deadline: std::time::Instant,
    ) -> Result<()> {
        checkpoint::export(self, target, expected, deadline)
    }
}
