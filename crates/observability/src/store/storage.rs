//! **本 crate 唯一允许做真实 IO 的文件。**
//!
//! 每个私有根下只碰一对固定名字：死人开关是 `deadman-state.json` 与暂存件
//! `deadman-state.json.new`，出站门铃是 `doorbell-state.json` 与
//! `doorbell-state.json.new`（两者各用各的私有目录）。逐段走 `NOFOLLOW` 打开父目录，
//! 核对目录 0700、文件 0600、当前 EUID 与单链接；写入走"暂存件 → fsync → renameat →
//! fsync 目录"的完整替换，读侧再靠文档内校验和判定撕裂，两层一起保证半写一律失败关闭。
//!
//! 暂存件的名字是固定的，所以两个进程不能同时替换同一份状态：后来的会删掉先到者的暂存件，
//! 再被对方把自己的空文件改名成正式状态。整段替换拿目录上的 `flock` 串行；门铃的存储
//! 从打开到放下一直独占这把锁，同一个状态目录同一时间只有一个门铃进程在跑。
//!
//! 不对抗可信父目录被同 UID 管理员恶意更名的 ABA；不导出任何文件能力。

use super::{StoreError, doorbell_state, state};
use crate::{alert::DeadmanState, doorbell::DoorbellState};
use std::path::{Path, PathBuf};

/// 固定状态文件名。
pub const STATE_FILE_NAME: &str = "deadman-state.json";
/// 固定暂存文件名；只用于完整替换，不是第二份状态。
pub const STAGE_FILE_NAME: &str = "deadman-state.json.new";
/// 出站门铃的固定状态文件名。
pub const DOORBELL_STATE_FILE_NAME: &str = "doorbell-state.json";
/// 出站门铃的固定暂存文件名；只用于完整替换。
pub const DOORBELL_STAGE_FILE_NAME: &str = "doorbell-state.json.new";

/// 一份持久文档在私有根下的固定名字对。
#[cfg(unix)]
#[derive(Clone, Copy)]
struct Names {
    state: &'static str,
    stage: &'static str,
}

#[cfg(unix)]
const DEADMAN: Names = Names {
    state: STATE_FILE_NAME,
    stage: STAGE_FILE_NAME,
};
#[cfg(unix)]
const DOORBELL: Names = Names {
    state: DOORBELL_STATE_FILE_NAME,
    stage: DOORBELL_STAGE_FILE_NAME,
};

#[cfg(unix)]
use rustix::fs::{
    AtFlags, FileType, FlockOperation, Mode, OFlags, flock, fstat, fsync, open, openat, renameat,
    unlinkat,
};

/// 绑定到一个已存在私有目录的死人开关状态存储。
pub struct DeadmanStore {
    #[cfg(unix)]
    root: Root,
    #[cfg(not(unix))]
    _unsupported: PhantomRoot,
}

/// 绑定到一个已存在私有目录的出站门铃状态存储（读到账本第几条、退避到什么时候）。
pub struct DoorbellStore {
    #[cfg(unix)]
    root: Root,
    #[cfg(not(unix))]
    _unsupported: PhantomRoot,
}

#[cfg(not(unix))]
struct PhantomRoot;

/// 已核对身份与权限的私有目录句柄；两种状态存储共用同一套读写。
#[cfg(unix)]
struct Root {
    root: rustix::fd::OwnedFd,
    root_path: PathBuf,
    owner: u32,
    /// 打开时就独占了目录锁、一直持到放下；为假时每次替换各拿一次。
    exclusive: bool,
}

/// 替换期间持有的目录锁；放下即解锁。
#[cfg(unix)]
struct Serial<'a>(&'a rustix::fd::OwnedFd);

#[cfg(unix)]
impl Drop for Serial<'_> {
    fn drop(&mut self) {
        let _ = flock(self.0, FlockOperation::Unlock);
    }
}

/// 门铃独占的目录锁跟着打开的文件描述走：同进程别的线程起子进程时，子进程到 exec 之前一直
/// 拿着这个描述的副本，只关本 FD 会让锁活到子进程 exec，紧接着再打开就报 `Busy`（BUG-041）。
/// 放下时显式解锁，立即让位；没独占时本描述上平常没有锁（替换期间的锁由 `Serial` 放）。
#[cfg(unix)]
impl Drop for Root {
    fn drop(&mut self) {
        if self.exclusive {
            let _ = flock(&self.root, FlockOperation::Unlock);
        }
    }
}

#[cfg(unix)]
impl DeadmanStore {
    /// 绑定已存在的私有目录；不创建目录，也不修复权限。
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            root: Root::open(root, false)?,
        })
    }

    /// 读取持久状态；固定文件不存在返回 `Ok(None)`（首次运行）。
    ///
    /// 撕裂、未知版本、未知字段、超限一律返回错误，**不会**降级成 `Ok(None)`。
    pub fn load(&self) -> Result<Option<DeadmanState>, StoreError> {
        self.root
            .read(DEADMAN)?
            .map(|bytes| state::decode(&bytes))
            .transpose()
    }

    /// 完整替换持久状态。中途失败只会留下暂存件，不会留下半个状态文件。
    pub fn save(&self, value: &DeadmanState) -> Result<(), StoreError> {
        self.root.replace(DEADMAN, &state::encode(value)?)
    }

    /// 重新核对根目录身份与权限；每次读写前后各做一次。
    pub fn check(&self) -> Result<(), StoreError> {
        self.root.check()
    }
}

#[cfg(unix)]
impl DoorbellStore {
    /// 绑定已存在的私有目录并独占它，直到本值放下；不创建目录，也不修复权限。
    ///
    /// 另一个进程正开着同一个目录时返回 [`StoreError::Busy`]，不等待：定时器和手动运行撞上时，
    /// 后到的那个直接让开，不会两边各读各写、互相盖掉位置和退避。
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            root: Root::open(root, true)?,
        })
    }

    /// 读取持久状态；固定文件不存在返回 `Ok(None)`（还没启用过）。
    ///
    /// 撕裂、未知版本、未知字段、超限一律返回错误，**不会**降级成 `Ok(None)`。
    pub fn load(&self) -> Result<Option<DoorbellState>, StoreError> {
        self.root
            .read(DOORBELL)?
            .map(|bytes| doorbell_state::decode(&bytes))
            .transpose()
    }

    /// 完整替换持久状态。中途失败只会留下暂存件，不会留下半个状态文件。
    pub fn save(&self, value: &DoorbellState) -> Result<(), StoreError> {
        self.root.replace(DOORBELL, &doorbell_state::encode(value)?)
    }

    /// 重新核对根目录身份与权限。
    pub fn check(&self) -> Result<(), StoreError> {
        self.root.check()
    }
}

#[cfg(unix)]
impl Root {
    fn open(root: &Path, exclusive: bool) -> Result<Self, StoreError> {
        let owner = caller()?;
        let handle = walk(root)?;
        directory(&handle, owner)?;
        if exclusive {
            match flock(&handle, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => {}
                Err(rustix::io::Errno::WOULDBLOCK) => return Err(StoreError::Busy),
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        Ok(Self {
            root: handle,
            root_path: root.to_owned(),
            owner,
            exclusive,
        })
    }

    /// 没有整段独占时，替换前等到目录锁；锁随返回值放下。
    fn serial(&self) -> Result<Option<Serial<'_>>, StoreError> {
        if self.exclusive {
            return Ok(None);
        }
        loop {
            match flock(&self.root, FlockOperation::LockExclusive) {
                Ok(()) => return Ok(Some(Serial(&self.root))),
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
    }

    /// 读出固定状态文件的全部字节；不存在返回 `Ok(None)`。
    fn read(&self, names: Names) -> Result<Option<Vec<u8>>, StoreError> {
        self.check()?;
        let file = match openat(
            &self.root,
            names.state,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(handle) => handle,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(open_error(error)),
        };
        let length = regular(&file, self.owner)?;
        let mut bytes = vec![0_u8; length];
        let mut filled = 0;
        while filled < length {
            match rustix::io::read(&file, &mut bytes[filled..]) {
                Ok(0) => return Err(StoreError::Corrupt),
                Ok(count) => filled += count,
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        // 读完约定长度后还有内容，说明文件在读取期间被改写；不接受这种证据。
        let mut tail = [0_u8; 1];
        match rustix::io::read(&file, &mut tail[..]) {
            Ok(0) => {}
            Ok(_) => return Err(StoreError::Corrupt),
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(StoreError::Unavailable),
        }
        self.check()?;
        Ok(Some(bytes))
    }

    /// 暂存件 → fsync → renameat → fsync 目录；中途失败只留下暂存件。
    fn replace(&self, names: Names, bytes: &[u8]) -> Result<(), StoreError> {
        let _serial = self.serial()?;
        self.check()?;
        // 上一轮崩在中途留下的暂存件必须先清掉，再独占创建，避免复用陌生对象。
        match unlinkat(&self.root, names.stage, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(StoreError::UnsafeStorage),
        }
        let stage = openat(
            &self.root,
            names.stage,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(open_error)?;
        let mut written = 0;
        while written < bytes.len() {
            match rustix::io::write(&stage, &bytes[written..]) {
                Ok(0) => return Err(StoreError::Unavailable),
                Ok(count) => written += count,
                Err(rustix::io::Errno::INTR) => {}
                Err(_) => return Err(StoreError::Unavailable),
            }
        }
        fsync(&stage).map_err(|_| StoreError::Unavailable)?;
        self.check()?;
        renameat(&self.root, names.stage, &self.root, names.state)
            .map_err(|_| StoreError::Unavailable)?;
        fsync(&self.root).map_err(|_| StoreError::Unavailable)?;
        self.check()
    }

    /// 重新核对根目录身份与权限；每次读写前后各做一次。
    fn check(&self) -> Result<(), StoreError> {
        if caller()? != self.owner {
            return Err(StoreError::UnsafeStorage);
        }
        let current = walk(&self.root_path)?;
        directory(&current, self.owner)?;
        let left = fstat(&current).map_err(|_| StoreError::UnsafeStorage)?;
        let right = fstat(&self.root).map_err(|_| StoreError::UnsafeStorage)?;
        if (left.st_dev, left.st_ino) != (right.st_dev, right.st_ino) {
            return Err(StoreError::UnsafeStorage);
        }
        Ok(())
    }
}

#[cfg(unix)]
fn caller() -> Result<u32, StoreError> {
    use rustix::process::{getegid, geteuid, getgid, getuid};
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(StoreError::UnsafeStorage);
    }
    Ok(geteuid().as_raw())
}

#[cfg(unix)]
fn walk(path: &Path) -> Result<rustix::fd::OwnedFd, StoreError> {
    use std::path::Component;
    if !path.is_absolute() {
        return Err(StoreError::UnsafeStorage);
    }
    let flags =
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut handle = open("/", flags, Mode::empty()).map_err(open_error)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                handle = openat(&handle, name, flags, Mode::empty()).map_err(open_error)?;
            }
            _ => return Err(StoreError::UnsafeStorage),
        }
    }
    Ok(handle)
}

#[cfg(unix)]
fn directory(handle: &rustix::fd::OwnedFd, owner: u32) -> Result<(), StoreError> {
    let stat = fstat(handle).map_err(|_| StoreError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != owner
        || Mode::from_raw_mode(stat.st_mode) != Mode::from_raw_mode(0o700)
    {
        return Err(StoreError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(unix)]
fn regular(handle: &rustix::fd::OwnedFd, owner: u32) -> Result<usize, StoreError> {
    let stat = fstat(handle).map_err(|_| StoreError::UnsafeStorage)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_nlink != 1
        || stat.st_uid != owner
        || Mode::from_raw_mode(stat.st_mode) != Mode::from_raw_mode(0o600)
    {
        return Err(StoreError::UnsafeStorage);
    }
    let length = usize::try_from(stat.st_size).map_err(|_| StoreError::UnsafeStorage)?;
    if length > state::STATE_MAX_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(length)
}

#[cfg(unix)]
fn open_error(error: rustix::io::Errno) -> StoreError {
    match error {
        rustix::io::Errno::NOENT => StoreError::Missing,
        rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR | rustix::io::Errno::ACCESS => {
            StoreError::UnsafeStorage
        }
        _ => StoreError::Unavailable,
    }
}

#[cfg(not(unix))]
impl DeadmanStore {
    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn open(_root: &Path) -> Result<Self, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn load(&self) -> Result<Option<DeadmanState>, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn save(&self, _value: &DeadmanState) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn check(&self) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }
}

#[cfg(not(unix))]
impl DoorbellStore {
    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn open(_root: &Path) -> Result<Self, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn load(&self) -> Result<Option<DoorbellState>, StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn save(&self, _value: &DoorbellState) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }

    /// 当前平台没有实现安全的私有本地文件约束。
    pub fn check(&self) -> Result<(), StoreError> {
        Err(StoreError::UnsupportedPlatform)
    }
}

#[cfg(all(test, unix))]
#[path = "../../tests/private/store_lock.rs"]
mod tests;
