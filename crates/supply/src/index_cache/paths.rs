//! 缓存只有固定文件写入能力；逐层拒绝链接并在每次操作前后核验绑定。

use std::path::{Path, PathBuf};

use crate::SupplyError;

pub(super) struct Storage {
    pub(super) database: PathBuf,
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
    pub(super) fn prepare(root: &Path) -> Result<(Self, bool), SupplyError> {
        use std::{
            fs::{self, OpenOptions},
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let metadata = directory(root)?;
        let owner = metadata.uid();
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(
                &root.join(format!("index.sqlite3{suffix}")),
                owner,
                metadata.dev(),
                true,
            )?;
        }
        let database = root.join("index.sqlite3");
        let created = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&database)
        {
            Ok(file) => {
                file.sync_all().map_err(|_| SupplyError::StoreUnavailable)?;
                fs::File::open(root)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| SupplyError::StoreUnavailable)?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(_) => return Err(SupplyError::StoreUnavailable),
        };
        let db = inspect_file(&database, owner, metadata.dev(), false)?
            .ok_or(SupplyError::UnsafeStorage)?;
        if !created && db.len() == 0 {
            return Err(SupplyError::CorruptStore);
        }
        let storage = Self {
            database,
            root: root.to_owned(),
            owner,
            root_identity: (metadata.dev(), metadata.ino()),
            database_identity: (db.dev(), db.ino()),
        };
        storage.check()?;
        Ok((storage, created))
    }

    pub(super) fn check(&self) -> Result<(), SupplyError> {
        use std::os::unix::fs::MetadataExt;
        let root = directory(&self.root)?;
        let db = inspect_file(&self.database, self.owner, root.dev(), false)?
            .ok_or(SupplyError::UnsafeStorage)?;
        if (root.dev(), root.ino()) != self.root_identity
            || root.uid() != self.owner
            || (db.dev(), db.ino()) != self.database_identity
        {
            return Err(SupplyError::UnsafeStorage);
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            inspect_file(
                &self.root.join(format!("index.sqlite3{suffix}")),
                self.owner,
                root.dev(),
                true,
            )?;
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl Storage {
    pub(super) fn prepare(_root: &Path) -> Result<(Self, bool), SupplyError> {
        Err(SupplyError::UnsafeStorage)
    }

    pub(super) fn check(&self) -> Result<(), SupplyError> {
        Err(SupplyError::UnsafeStorage)
    }
}

#[cfg(unix)]
fn directory(root: &Path) -> Result<std::fs::Metadata, SupplyError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = directory_components(root)?;
    if metadata.mode() & 0o7777 != 0o700 {
        return Err(SupplyError::UnsafeStorage);
    }
    Ok(metadata)
}

#[cfg(unix)]
fn directory_components(root: &Path) -> Result<std::fs::Metadata, SupplyError> {
    use std::{fs, path::Component};
    if !root.is_absolute() {
        return Err(SupplyError::UnsafeStorage);
    }
    let mut current = PathBuf::new();
    for component in root.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(SupplyError::UnsafeStorage);
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| SupplyError::UnsafeStorage)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SupplyError::UnsafeStorage);
        }
    }
    fs::symlink_metadata(root).map_err(|_| SupplyError::UnsafeStorage)
}

#[cfg(unix)]
pub(super) fn validate_isolation(root: &Path, forbidden: &[&Path]) -> Result<(), SupplyError> {
    use std::{fs, os::unix::fs::MetadataExt};
    let root_info = directory(root)?;
    let actual_root = fs::canonicalize(root).map_err(|_| SupplyError::UnsafeStorage)?;
    let resolved_root = directory(&actual_root)?;
    if (root_info.dev(), root_info.ino()) != (resolved_root.dev(), resolved_root.ino()) {
        return Err(SupplyError::UnsafeStorage);
    }
    for other in forbidden {
        let other_info = directory_components(other)?;
        let actual_other = fs::canonicalize(other).map_err(|_| SupplyError::UnsafeStorage)?;
        let resolved_other = directory_components(&actual_other)?;
        let other_after = directory_components(other)?;
        if actual_root.starts_with(&actual_other)
            || actual_other.starts_with(&actual_root)
            || (root_info.dev(), root_info.ino()) == (other_info.dev(), other_info.ino())
            || (other_info.dev(), other_info.ino()) != (resolved_other.dev(), resolved_other.ino())
            || (other_info.dev(), other_info.ino()) != (other_after.dev(), other_after.ino())
        {
            return Err(SupplyError::UnsafeStorage);
        }
    }
    // 规范化和比较期间重绑定也不允许通过启动检查。
    let after = directory(root)?;
    if (root_info.dev(), root_info.ino()) != (after.dev(), after.ino()) {
        return Err(SupplyError::UnsafeStorage);
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_isolation(_root: &Path, _forbidden: &[&Path]) -> Result<(), SupplyError> {
    Err(SupplyError::UnsafeStorage)
}

#[cfg(unix)]
fn inspect_file(
    path: &Path,
    owner: u32,
    device: u64,
    optional: bool,
) -> Result<Option<std::fs::Metadata>, SupplyError> {
    use std::{fs, os::unix::fs::MetadataExt};
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.nlink() != 1
                || metadata.mode() & 0o7777 != 0o600
                || metadata.uid() != owner
                || metadata.dev() != device
            {
                return Err(SupplyError::UnsafeStorage);
            }
            Ok(Some(metadata))
        }
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(SupplyError::UnsafeStorage),
    }
}
