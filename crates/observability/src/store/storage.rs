//! **本 crate 唯一允许做真实 IO 的文件。**
//!
//! 固定私有根下只碰两个固定名字：`deadman-state.json` 与暂存件
//! `deadman-state.json.new`。逐段走 `NOFOLLOW` 打开父目录，核对目录 0700、
//! 文件 0600、当前 EUID 与单链接；写入走"暂存件 → fsync → renameat → fsync 目录"
//! 的完整替换，读侧再靠文档内校验和判定撕裂，两层一起保证半写一律失败关闭。
//!
//! 不对抗可信父目录被同 UID 管理员恶意更名的 ABA；不导出任何文件能力。

use super::{StoreError, state};
use crate::alert::DeadmanState;
use std::path::{Path, PathBuf};

/// 固定状态文件名。
pub const STATE_FILE_NAME: &str = "deadman-state.json";
/// 固定暂存文件名；只用于完整替换，不是第二份状态。
pub const STAGE_FILE_NAME: &str = "deadman-state.json.new";

#[cfg(unix)]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, fsync, open, openat, renameat, unlinkat};

/// 绑定到一个已存在私有目录的死人开关状态存储。
pub struct DeadmanStore {
    #[cfg(unix)]
    root: rustix::fd::OwnedFd,
    #[cfg(unix)]
    root_path: PathBuf,
    #[cfg(unix)]
    owner: u32,
    #[cfg(not(unix))]
    _unsupported: PhantomRoot,
}

#[cfg(not(unix))]
struct PhantomRoot;

#[cfg(unix)]
impl DeadmanStore {
    /// 绑定已存在的私有目录；不创建目录，也不修复权限。
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let owner = caller()?;
        let handle = walk(root)?;
        directory(&handle, owner)?;
        Ok(Self {
            root: handle,
            root_path: root.to_owned(),
            owner,
        })
    }

    /// 读取持久状态；固定文件不存在返回 `Ok(None)`（首次运行）。
    ///
    /// 撕裂、未知版本、未知字段、超限一律返回错误，**不会**降级成 `Ok(None)`。
    pub fn load(&self) -> Result<Option<DeadmanState>, StoreError> {
        self.check()?;
        let file = match openat(
            &self.root,
            STATE_FILE_NAME,
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
        state::decode(&bytes).map(Some)
    }

    /// 完整替换持久状态。中途失败只会留下暂存件，不会留下半个状态文件。
    pub fn save(&self, value: &DeadmanState) -> Result<(), StoreError> {
        self.check()?;
        let bytes = state::encode(value)?;
        // 上一轮崩在中途留下的暂存件必须先清掉，再独占创建，避免复用陌生对象。
        match unlinkat(&self.root, STAGE_FILE_NAME, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(StoreError::UnsafeStorage),
        }
        let stage = openat(
            &self.root,
            STAGE_FILE_NAME,
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
        renameat(&self.root, STAGE_FILE_NAME, &self.root, STATE_FILE_NAME)
            .map_err(|_| StoreError::Unavailable)?;
        fsync(&self.root).map_err(|_| StoreError::Unavailable)?;
        self.check()
    }

    /// 重新核对根目录身份与权限；每次读写前后各做一次。
    pub fn check(&self) -> Result<(), StoreError> {
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
