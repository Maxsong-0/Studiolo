//! 黑板控制数据目录独立于内容、身份与策略；启动不修复权限。
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

pub(super) struct Storage {
    _directory_pin: fs::File,
    _file_pin: fs::File,
    directory: PathBuf,
    identity: (u64, u64),
    file: PathBuf,
    file_identity: (u64, u64),
}
impl Storage {
    pub(super) fn open(directory: &Path, isolated: &[&Path]) -> Result<Self, &'static str> {
        if !directory.is_absolute()
            || fs::canonicalize(directory).map_err(|_| "控制目录不可用")? != directory
        {
            return Err("控制目录必须为规范绝对路径");
        }
        for other in isolated {
            let other = fs::canonicalize(other).map_err(|_| "隔离根不可用")?;
            if directory.starts_with(&other) || other.starts_with(directory) {
                return Err("控制目录必须独立");
            }
        }
        let metadata = fs::symlink_metadata(directory).map_err(|_| "控制目录不可用")?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err("控制目录必须为同 UID 的 0700 私有目录");
        }
        let file = directory.join("control.sqlite3");
        if !file.try_exists().map_err(|_| "控制库不可用")? {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&file)
                .map_err(|_| "控制库创建失败")?;
        }
        let fm = fs::symlink_metadata(&file).map_err(|_| "控制库不可用")?;
        let storage = Self {
            _directory_pin: fs::File::open(directory).map_err(|_| "控制目录不可用")?,
            _file_pin: fs::File::open(&file).map_err(|_| "控制库不可用")?,
            directory: directory.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
            file,
            file_identity: (fm.dev(), fm.ino()),
        };
        storage.check()?;
        Ok(storage)
    }
    pub(super) fn file(&self) -> &Path {
        &self.file
    }
    pub(super) fn check(&self) -> Result<(), &'static str> {
        let d = fs::symlink_metadata(&self.directory).map_err(|_| "控制目录不可用")?;
        let f = fs::symlink_metadata(&self.file).map_err(|_| "控制库不可用")?;
        let uid = rustix::process::geteuid().as_raw();
        let dp = self
            ._directory_pin
            .metadata()
            .map_err(|_| "控制目录不可用")?;
        let fp = self._file_pin.metadata().map_err(|_| "控制库不可用")?;
        if (dp.dev(), dp.ino()) != self.identity
            || (fp.dev(), fp.ino()) != self.file_identity
            || fs::canonicalize(&self.directory).map_err(|_| "控制目录不可用")? != self.directory
            || !d.is_dir()
            || (d.dev(), d.ino()) != self.identity
            || d.uid() != uid
            || d.permissions().mode() & 0o777 != 0o700
            || !f.is_file()
            || (f.dev(), f.ino()) != self.file_identity
            || f.uid() != uid
            || f.nlink() != 1
            || f.permissions().mode() & 0o777 != 0o600
        {
            return Err("控制存储身份或权限变化");
        }
        Ok(())
    }
}
