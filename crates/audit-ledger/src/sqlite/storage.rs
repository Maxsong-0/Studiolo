//! 仅此文件拥有审计私有目录的文件权限；不接受 Vault 能力或任意数据文件名。

use crate::AuditError;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(super) enum CreatePolicy {
    AllowNew,
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
    pub(super) fn prepare(root: &Path, policy: CreatePolicy) -> Result<Self, AuditError> {
        use std::{
            fs::{self, OpenOptions},
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let metadata = directory(root)?;
        let owner = metadata.uid();
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(&root.join(format!("audit.sqlite3{suffix}")), owner, true)?;
        }
        let database = root.join("audit.sqlite3");
        let created = match policy {
            CreatePolicy::ExistingOnly => false,
            CreatePolicy::AllowNew => match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&database)
            {
                Ok(file) => {
                    file.sync_all().map_err(|_| AuditError::Unavailable)?;
                    fs::File::open(root)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| AuditError::Unavailable)?;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(_) => return Err(AuditError::Unavailable),
            },
        };
        let missing = match policy {
            CreatePolicy::AllowNew => AuditError::UnsafeStorage,
            CreatePolicy::ExistingOnly => AuditError::MissingStore,
        };
        let db = inspect_file(&database, owner, true)?.ok_or(missing)?;
        if !created && db.len() == 0 {
            return Err(AuditError::CorruptStore);
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

    pub(super) fn check(&self) -> Result<(), AuditError> {
        use std::os::unix::fs::MetadataExt;
        let root = directory(&self.root)?;
        let db =
            inspect_file(&self.database, self.owner, false)?.ok_or(AuditError::UnsafeStorage)?;
        if (root.dev(), root.ino()) != self.root_identity
            || root.uid() != self.owner
            || (db.dev(), db.ino()) != self.database_identity
        {
            return Err(AuditError::UnsafeStorage);
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(
                &self.root.join(format!("audit.sqlite3{suffix}")),
                self.owner,
                true,
            )?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn directory(root: &Path) -> Result<std::fs::Metadata, AuditError> {
    use std::{fs, os::unix::fs::MetadataExt, path::Component};
    if !root.is_absolute() {
        return Err(AuditError::UnsafeStorage);
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(AuditError::UnsafeStorage);
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| AuditError::UnsafeStorage)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AuditError::UnsafeStorage);
        }
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| AuditError::UnsafeStorage)?;
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(AuditError::UnsafeStorage);
    }
    Ok(metadata)
}

#[cfg(unix)]
fn inspect_file(
    path: &Path,
    owner: u32,
    optional: bool,
) -> Result<Option<std::fs::Metadata>, AuditError> {
    use std::{fs, os::unix::fs::MetadataExt};
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.nlink() != 1
                || metadata.mode() & 0o7777 != 0o600
                || metadata.uid() != owner
            {
                return Err(AuditError::UnsafeStorage);
            }
            Ok(Some(metadata))
        }
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(AuditError::UnsafeStorage),
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(super) fn prepare(_root: &Path, _policy: CreatePolicy) -> Result<Self, AuditError> {
        Err(AuditError::UnsupportedPlatform)
    }
    pub(super) fn check(&self) -> Result<(), AuditError> {
        Err(AuditError::UnsupportedPlatform)
    }
}
