//! Private repository-identity leases and immutable landing intent/outcome records.
use super::hash;
use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(super) struct Lease {
    _file: File,
    _repository: File,
    pub identity: String,
    root: PathBuf,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // A concurrent fork may retain this open file description until exec.
        // End our lease now instead of waiting for every inherited descriptor to close.
        let _ = flock(&self._file, FlockOperation::Unlock);
    }
}
impl Lease {
    #[cfg(test)]
    pub(super) fn duplicate_descriptor_for_test(&self) -> std::io::Result<File> {
        self._file.try_clone()
    }
    pub fn acquire(root: &Path, repository: &Path) -> Result<Self, String> {
        super::config::private_directory(root)?;
        if fs::canonicalize(repository).map_err(|_| "仓库不可用")? != repository {
            return Err("仓库路径身份变化".into());
        }
        let directory = File::from(
            open(
                repository,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| "仓库不可用")?,
        );
        let meta = directory.metadata().map_err(|_| "仓库身份不可用")?;
        if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o022 != 0 {
            return Err("仓库必须属于本UID且禁止组或其他用户写入".into());
        }
        let identity = hash(format!("{}:{}", meta.dev(), meta.ino()).as_bytes());
        let path = root.join(format!("landing-{identity}.lock"));
        let file = File::from(
            open(
                &path,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| "仓库落地锁不可用")?,
        );
        private(&file)?;
        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|_| "仓库正在快照或落地")?;
        let lease = Self {
            _file: file,
            _repository: directory,
            identity,
            root: root.to_owned(),
        };
        let current = fs::symlink_metadata(&path).map_err(|_| "仓库锁身份不可用")?;
        let held = lease._file.metadata().map_err(|_| "仓库锁身份不可用")?;
        if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
            return Err("仓库锁身份变化".into());
        }
        Ok(lease)
    }
    fn path(&self, key: &str, phase: &str) -> PathBuf {
        // Attempt identity stays globally unique even if an administrator replaces its repository.
        // The record itself pins repository_identity, so replacement cannot make a prior intent vanish.
        self.root.join(format!("landing-{key}-{phase}.json"))
    }
    pub fn verify_repository(&self, repository: &Path) -> Result<(), String> {
        let meta = fs::symlink_metadata(repository).map_err(|_| "仓库身份不可用")?;
        if !meta.is_dir()
            || hash(format!("{}:{}", meta.dev(), meta.ino()).as_bytes()) != self.identity
        {
            return Err("仓库身份变化".into());
        }
        Ok(())
    }
    pub fn read<T: DeserializeOwned>(&self, key: &str, phase: &str) -> Result<Option<T>, String> {
        let file = match open(
            self.path(key, phase),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err("落地记录不可用".into()),
        };
        private(&file)?;
        let mut bytes = Vec::new();
        file.take(131073)
            .read_to_end(&mut bytes)
            .map_err(|_| "落地记录读取失败")?;
        if bytes.len() > 131072 {
            return Err("落地记录越界".into());
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "落地记录损坏".into())
    }
    pub fn write<T: Serialize>(&self, key: &str, phase: &str, value: &T) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|_| "落地记录编码失败")?;
        if bytes.len() > 131072 {
            return Err("落地记录越界".into());
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(self.path(key, phase))
            .map_err(|_| "落地记录创建失败")?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| "落地记录持久化失败")?;
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|_| "落地目录持久化失败".into())
    }
}
fn private(file: &File) -> Result<(), String> {
    let m = file.metadata().map_err(|_| "落地控制文件不可用")?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.mode() & 0o777 != 0o600
    {
        return Err("落地控制文件必须为同UID的0600普通文件".into());
    }
    Ok(())
}
