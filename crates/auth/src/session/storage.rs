//! 仅此文件拥有认证私有目录的文件权限；不接受 Vault 能力或任意数据文件名。

use super::SessionError;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CreatePolicy {
    Allow,
    ExistingOnly,
}

pub(super) struct Storage {
    pub(super) database: PathBuf,
    pub(super) created: bool,
    #[cfg(unix)]
    root: PathBuf,
    #[cfg(unix)]
    root_identity: (u64, u64),
    #[cfg(unix)]
    database_identity: (u64, u64),
    #[cfg(unix)]
    owner: u32,
}

#[cfg(unix)]
impl Storage {
    pub(super) fn prepare(root: &Path, create: CreatePolicy) -> Result<Self, SessionError> {
        use std::{
            fs::{self, OpenOptions},
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let metadata = directory(root)?;
        let owner = metadata.uid();
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(&root.join(format!("auth.sqlite3{suffix}")), owner, true)?;
        }
        let database = root.join("auth.sqlite3");
        // 只允许显式创建路径进入 create_new；不存在的普通打开绝不补建空文件。
        let created = if create == CreatePolicy::Allow {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&database)
            {
                Ok(file) => {
                    file.sync_all()
                        .map_err(|_| SessionError::StorageUnavailable)?;
                    fs::File::open(root)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| SessionError::StorageUnavailable)?;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(_) => return Err(SessionError::StorageUnavailable),
            }
        } else {
            false
        };
        let db = inspect_file(&database, owner, create == CreatePolicy::ExistingOnly)?
            .ok_or(SessionError::MissingStore)?;
        if !created && db.len() == 0 {
            return Err(SessionError::CorruptStore);
        }
        let storage = Self {
            database,
            created,
            root: root.to_owned(),
            owner,
            root_identity: (metadata.dev(), metadata.ino()),
            database_identity: (db.dev(), db.ino()),
        };
        storage.check()?;
        Ok(storage)
    }

    pub(super) fn check(&self) -> Result<(), SessionError> {
        use std::os::unix::fs::MetadataExt;
        let root = directory(&self.root)?;
        let db =
            inspect_file(&self.database, self.owner, false)?.ok_or(SessionError::UnsafeStorage)?;
        if (root.dev(), root.ino()) != self.root_identity
            || root.uid() != self.owner
            || (db.dev(), db.ino()) != self.database_identity
        {
            return Err(SessionError::UnsafeStorage);
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(
                &self.root.join(format!("auth.sqlite3{suffix}")),
                self.owner,
                true,
            )?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn directory(root: &Path) -> Result<std::fs::Metadata, SessionError> {
    use std::{fs, os::unix::fs::MetadataExt, path::Component};
    if !root.is_absolute() {
        return Err(SessionError::UnsafeStorage);
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(SessionError::UnsafeStorage);
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| SessionError::UnsafeStorage)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SessionError::UnsafeStorage);
        }
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| SessionError::UnsafeStorage)?;
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(SessionError::UnsafeStorage);
    }
    Ok(metadata)
}

#[cfg(unix)]
fn inspect_file(
    path: &Path,
    owner: u32,
    optional: bool,
) -> Result<Option<std::fs::Metadata>, SessionError> {
    use std::{fs, os::unix::fs::MetadataExt};
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.nlink() != 1
                || metadata.mode() & 0o7777 != 0o600
                || metadata.uid() != owner
            {
                return Err(SessionError::UnsafeStorage);
            }
            Ok(Some(metadata))
        }
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(SessionError::UnsafeStorage),
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(super) fn prepare(_root: &Path, _create: CreatePolicy) -> Result<Self, SessionError> {
        Err(SessionError::UnsupportedPlatform)
    }
    pub(super) fn check(&self) -> Result<(), SessionError> {
        Err(SessionError::UnsupportedPlatform)
    }
}

#[cfg(test)]
#[path = "../../tests/session/storage.rs"]
mod tests;
