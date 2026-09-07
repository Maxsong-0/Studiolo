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
        // 不跟随末级链接；检查后若换成FIFO也不会在打开时无限等待。
        let descriptor = open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ConfigError)?;
        let file = File::from(descriptor);
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
