//! 仅此文件拥有 策略私有目录的文件权限；规则与审计私有存储一致（绝对路径、逐层拒绝链接、
//! 目录 0700、文件 0600/单链接/同所有者），只创建固定文件名 `policy.sqlite3`。

use std::path::{Path, PathBuf};

use crate::PolicyError;

#[derive(Clone, Copy)]
pub(crate) enum CreatePolicy {
    AllowNew,
    ExistingOnly,
}

pub(crate) struct Storage {
    pub(crate) database: PathBuf,
    pub(crate) created: bool,
    #[cfg(unix)]
    created_pin: Option<std::fs::File>,
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
    pub(crate) fn prepare(root: &Path, policy: CreatePolicy) -> Result<Self, PolicyError> {
        use std::{
            fs::{self, OpenOptions},
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let metadata = directory(root)?;
        let owner = metadata.uid();
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(&root.join(format!("policy.sqlite3{suffix}")), owner, true)?;
        }
        let database = root.join("policy.sqlite3");
        let created_pin = match policy {
            CreatePolicy::ExistingOnly => None,
            CreatePolicy::AllowNew => match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&database)
            {
                Ok(file) => {
                    file.sync_all().map_err(|_| PolicyError::StoreUnavailable)?;
                    fs::File::open(root)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| PolicyError::StoreUnavailable)?;
                    Some(file)
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(_) => return Err(PolicyError::StoreUnavailable),
            },
        };
        let missing = match policy {
            CreatePolicy::AllowNew => PolicyError::UnsafeStorage,
            CreatePolicy::ExistingOnly => PolicyError::MissingStore,
        };
        let db = inspect_file(&database, owner, true)?.ok_or(missing)?;
        if created_pin.is_none() && db.len() == 0 {
            return Err(PolicyError::CorruptStore);
        }
        let storage = Self {
            database,
            created: created_pin.is_some(),
            created_pin,
            root: root.to_owned(),
            owner,
            root_identity: (metadata.dev(), metadata.ino()),
            database_identity: (db.dev(), db.ino()),
        };
        storage.check()?;
        Ok(storage)
    }

    pub(crate) fn check(&self) -> Result<(), PolicyError> {
        use std::os::unix::fs::MetadataExt;
        let root = directory(&self.root)?;
        let db =
            inspect_file(&self.database, self.owner, false)?.ok_or(PolicyError::UnsafeStorage)?;
        if (root.dev(), root.ino()) != self.root_identity
            || root.uid() != self.owner
            || (db.dev(), db.ino()) != self.database_identity
        {
            return Err(PolicyError::UnsafeStorage);
        }
        // create_new 的证明必须来自仍持有的原 FD，不能在打开 SQLite 前收养替换文件。
        if let Some(pin) = &self.created_pin {
            let created = pin.metadata().map_err(|_| PolicyError::UnsafeStorage)?;
            if (created.dev(), created.ino()) != self.database_identity || created.nlink() != 1 {
                return Err(PolicyError::UnsafeStorage);
            }
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(
                &self.root.join(format!("policy.sqlite3{suffix}")),
                self.owner,
                true,
            )?;
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(crate) fn prepare(_root: &Path, _policy: CreatePolicy) -> Result<Self, PolicyError> {
        Err(PolicyError::UnsafeStorage)
    }

    pub(crate) fn check(&self) -> Result<(), PolicyError> {
        Err(PolicyError::UnsafeStorage)
    }
}

#[cfg(unix)]
fn directory(root: &Path) -> Result<std::fs::Metadata, PolicyError> {
    use std::{fs, os::unix::fs::MetadataExt, path::Component};
    if !root.is_absolute() {
        return Err(PolicyError::UnsafeStorage);
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(PolicyError::UnsafeStorage);
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| PolicyError::UnsafeStorage)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PolicyError::UnsafeStorage);
        }
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| PolicyError::UnsafeStorage)?;
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(PolicyError::UnsafeStorage);
    }
    Ok(metadata)
}

#[cfg(unix)]
fn inspect_file(
    path: &Path,
    owner: u32,
    optional: bool,
) -> Result<Option<std::fs::Metadata>, PolicyError> {
    use std::{fs, os::unix::fs::MetadataExt};
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.nlink() != 1
                || metadata.mode() & 0o7777 != 0o600
                || metadata.uid() != owner
            {
                return Err(PolicyError::UnsafeStorage);
            }
            Ok(Some(metadata))
        }
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(PolicyError::UnsafeStorage),
    }
}
