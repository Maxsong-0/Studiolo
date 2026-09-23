//! 唯一文件权限 Adapter：固定预算、对象身份和真实UID；无通用公有文件读取入口。

use crate::{ConfigError, absolute_normal};
use rustix::fs::{Mode, OFlags, open};
use rustix::process::{getegid, geteuid, getgid, getuid};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_CONFIG_BYTES: u64 = 16_384;
type Stamp = (u64, u64, u64, i64, i64, i64, i64);

/// 保存配置文件身份及最多16KiB原字节；重验不接受同路径的新内容或新对象。
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub path: PathBuf,
    stamp: Stamp,
    bytes: Arc<[u8]>,
}

impl Snapshot {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        if !absolute_normal(path) {
            return Err(ConfigError);
        }
        private_directory(path.parent().ok_or(ConfigError)?)?;
        let before = fs::symlink_metadata(path).map_err(|_| ConfigError)?;
        private_file(&before)?;
        let file = hold(path)?;
        let opened = file.metadata().map_err(|_| ConfigError)?;
        private_file(&opened)?;
        same_file(&before, &opened)?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ConfigError)?;
        if bytes.len() as u64 != opened.len() {
            return Err(ConfigError);
        }
        let after = fs::symlink_metadata(path).map_err(|_| ConfigError)?;
        private_file(&after)?;
        same_file(&opened, &after)?;
        private_directory(path.parent().ok_or(ConfigError)?)?;
        Ok(Self {
            path: path.to_path_buf(),
            stamp: stamp(&opened),
            bytes: bytes.into(),
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn revalidate(&self) -> Result<(), ConfigError> {
        let current = Self::read(&self.path)?;
        if self.stamp != current.stamp || self.bytes != current.bytes {
            return Err(ConfigError);
        }
        Ok(())
    }
}

/// 唯一打开入口：只读、不跟随末级链接、非阻塞、CLOEXEC；目录与普通文件共用。
///
/// 不跟随末级链接；检查后若换成FIFO也不会在打开时无限等待。目录也按同一组标志打开：
/// 打开后立刻用 `fstat` 判定类型，比 `O_DIRECTORY` 少一个标志但结论相同。
fn hold(path: &Path) -> Result<File, ConfigError> {
    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| ConfigError)?;
    Ok(File::from(descriptor))
}

/// 持有 FD 的私有根身份钉；即使 inode 号被复用也能认出「批准的那个对象已经没了」。
///
/// 只存 `(dev, ino)` 再按路径复比在 Linux 上挡不住替换：把目录 rmdir 后同名 mkdir，
/// 内核会把同一个 inode 号立刻发还（ext4 之上的 overlayfs 实测 20/20 命中），
/// 于是新目录与被批准的旧目录取值完全相同。持有 FD 之后有两条互相独立的信号：
///
/// 1. 对**持有的 FD** `fstat`：目录被 rmdir 后该 inode 的链接数归零，
///    inode 号随后被谁复用都改不了这一点（Linux 实测 20/20；APFS 不归零，见下）。
/// 2. FD 活着时内核不会把这个 inode 号发还给新对象（旧 inode 的引用没清零就不释放），
///    于是**按路径**取到的 `(dev, ino)` 与钉住的值不再相同（Linux、macOS 实测均成立）。
///
/// macOS/APFS 从不复用 inode 号、也不把链接数归零，由信号 2 独自覆盖；
/// Linux 两条同时成立。两条任一命中即判为漂移。
pub(crate) struct PinnedDirectory {
    path: PathBuf,
    descriptor: File,
    identity: (u64, u64),
}

impl PinnedDirectory {
    /// 先逐段核验路径是 0700、本人所有的真实目录，再持有它的 FD 并与路径身份互证。
    pub fn pin(path: &Path) -> Result<Self, ConfigError> {
        let expected = private_directory(path)?;
        let descriptor = hold(path)?;
        let pinned = descriptor.metadata().map_err(|_| ConfigError)?;
        private_node(&pinned)?;
        if identity(&pinned) != expected {
            return Err(ConfigError);
        }
        Ok(Self {
            path: path.to_path_buf(),
            descriptor,
            identity: expected,
        })
    }

    /// 批准时刻的 `(dev, ino)`；单独拿这个值比较不足以发现替换，复验必须走 `revalidate`。
    pub fn identity(&self) -> (u64, u64) {
        self.identity
    }

    /// 复验被钉住的对象仍然在，且这条路径仍然指向它。
    ///
    /// 成功只代表检查时刻一致；不能证明与后续 IO 的系统级原子性。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        let pinned = self.descriptor.metadata().map_err(|_| ConfigError)?;
        // 信号 1：被删掉的目录，其 inode 链接数归零；inode 号被新目录复用也盖不住。
        if pinned.nlink() == 0 {
            return Err(ConfigError);
        }
        private_node(&pinned)?;
        if identity(&pinned) != self.identity {
            return Err(ConfigError);
        }
        // 信号 2：这条路径现在指向的对象，必须仍是批准时钉住的那一个。
        if private_directory(&self.path)? != self.identity {
            return Err(ConfigError);
        }
        Ok(())
    }
}

pub(crate) fn directory(path: &Path) -> Result<(u64, u64), ConfigError> {
    if !absolute_normal(path) {
        return Err(ConfigError);
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        let metadata = fs::symlink_metadata(&current).map_err(|_| ConfigError)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ConfigError);
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ConfigError)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(ConfigError);
    }
    Ok(identity(&metadata))
}

pub(crate) fn private_directory(path: &Path) -> Result<(u64, u64), ConfigError> {
    if getuid() != geteuid() || getgid() != getegid() {
        return Err(ConfigError);
    }
    let checked = directory(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| ConfigError)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != getuid().as_raw()
        || identity(&metadata) != checked
    {
        return Err(ConfigError);
    }
    Ok(checked)
}

/// 词法之外再检查真实祖先身份，拒绝大小写别名导致的相同或嵌套根。
pub(crate) fn disjoint(left: &Path, right: &Path) -> Result<(), ConfigError> {
    let left_id = directory(left)?;
    let right_id = directory(right)?;
    for (path, forbidden) in [(left, right_id), (right, left_id)] {
        for parent in path.ancestors() {
            let metadata = fs::symlink_metadata(parent).map_err(|_| ConfigError)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || identity(&metadata) == forbidden
            {
                return Err(ConfigError);
            }
        }
    }
    Ok(())
}

pub(crate) fn outside(file: &Path, public_root: (u64, u64)) -> Result<(), ConfigError> {
    for parent in file.parent().ok_or(ConfigError)?.ancestors() {
        let metadata = fs::symlink_metadata(parent).map_err(|_| ConfigError)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || identity(&metadata) == public_root
        {
            return Err(ConfigError);
        }
    }
    Ok(())
}

fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

/// 已持有 FD 的对象本身必须仍是 0700、本人所有的真实目录。
fn private_node(metadata: &Metadata) -> Result<(), ConfigError> {
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.mode() & 0o7777 != 0o700
        || metadata.uid() != getuid().as_raw()
    {
        return Err(ConfigError);
    }
    Ok(())
}

fn private_file(metadata: &Metadata) -> Result<(), ConfigError> {
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o600
        || metadata.uid() != getuid().as_raw()
        || metadata.len() == 0
        || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(ConfigError);
    }
    Ok(())
}

fn stamp(metadata: &Metadata) -> Stamp {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

fn same_file(before: &Metadata, after: &Metadata) -> Result<(), ConfigError> {
    if stamp(before) != stamp(after) {
        return Err(ConfigError);
    }
    Ok(())
}
