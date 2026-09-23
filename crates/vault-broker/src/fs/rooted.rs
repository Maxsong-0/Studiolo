//! 有根打开与节点校验、原子安装与目录落盘；Linux 上用 openat2 做不跟随链接的解析、renameat2 做仅创建安装。

use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use cap_std::fs::{Dir, MetadataExt as _, OpenOptions};
use studiolo_core::VaultPath;

use crate::{BrokerError, FsVaultBroker};

#[cfg(target_os = "linux")]
use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2, renameat_with};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

impl FsVaultBroker {
    pub(crate) fn atomic_write_dir_checked(
        dir: &Dir,
        path: &Path,
        content: &[u8],
        create_new: bool,
        before_install: impl FnOnce() -> Result<(), BrokerError>,
    ) -> Result<(), BrokerError> {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(BrokerError::CorruptIdentity)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(
            ".{file_name}.studiolo-tmp-{}-{sequence}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut temp = dir
            .open_with(&temp_path, &options)
            .map_err(|error| BrokerError::io("create atomic temporary file", error))?;
        let write_result = (|| {
            temp.write_all(content)
                .map_err(|error| BrokerError::io("write atomic temporary file", error))?;
            temp.sync_all()
                .map_err(|error| BrokerError::io("sync atomic temporary file", error))?;
            before_install()?;
            if create_new {
                #[cfg(target_os = "linux")]
                renameat_with(dir, &temp_path, dir, path, RenameFlags::NOREPLACE)
                    .map_err(|error| BrokerError::io("install create-only file", error.into()))?;
                #[cfg(not(target_os = "linux"))]
                {
                    // link(2) 具备原子的仅创建语义；
                    // 先查元数据再用会覆盖目标的 rename(2) 则做不到。
                    dir.hard_link(&temp_path, dir, path)
                        .map_err(|error| BrokerError::io("install create-only file", error))?;
                    dir.remove_file(&temp_path).map_err(|error| {
                        BrokerError::io("unlink installed temporary file", error)
                    })?;
                }
            } else {
                dir.rename(&temp_path, dir, path)
                    .map_err(|error| BrokerError::io("rename atomic temporary file", error))?;
            }
            Self::sync_directory(dir, parent)
        })();
        if write_result.is_err() {
            let _ = dir.remove_file(&temp_path);
        }
        write_result
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn sync_directory(dir: &Dir, path: &Path) -> Result<(), BrokerError> {
        // cap-std 可能返回无法 fsync 的 O_PATH 目录句柄；
        // 这里在保持同一有根解析的前提下另取一个可读 fd。
        let handle = openat2(
            dir,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|error| BrokerError::io("open parent for sync", error.into()))?;
        std::fs::File::from(handle)
            .sync_all()
            .map_err(|error| BrokerError::io("sync parent directory", error))
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn sync_directory(dir: &Dir, path: &Path) -> Result<(), BrokerError> {
        dir.open_dir(path)
            .map_err(|error| BrokerError::io("open parent for sync", error))?
            .into_std_file()
            .sync_all()
            .map_err(|error| BrokerError::io("sync parent directory", error))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn open_directory(
        root: &Dir,
        directory: &Path,
        path: &VaultPath,
    ) -> Result<Dir, BrokerError> {
        let handle = openat2(
            root,
            directory,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|_| BrokerError::UnsafeNode {
            path: path.clone(),
            reason: "parent failed Linux beneath/no-link resolution",
        })?;
        Ok(Dir::from_std_file(std::fs::File::from(handle)))
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn open_directory(
        root: &Dir,
        directory: &Path,
        _path: &VaultPath,
    ) -> Result<Dir, BrokerError> {
        root.open_dir(directory)
            .map_err(|error| BrokerError::io("open rooted directory", error))
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn open_rooted_file(
        root: &Dir,
        path: &VaultPath,
    ) -> Result<cap_std::fs::File, BrokerError> {
        let handle = openat2(
            root,
            path.as_str(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        )
        .map_err(|error| {
            let error = std::io::Error::from(error);
            if error.kind() == std::io::ErrorKind::NotFound {
                BrokerError::NotFound(path.clone())
            } else {
                BrokerError::UnsafeNode {
                    path: path.clone(),
                    reason: "file failed Linux beneath/no-link resolution",
                }
            }
        })?;
        Ok(cap_std::fs::File::from_std(std::fs::File::from(handle)))
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn open_rooted_file(
        root: &Dir,
        path: &VaultPath,
    ) -> Result<cap_std::fs::File, BrokerError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            options.custom_flags(
                (rustix::fs::OFlags::NONBLOCK | rustix::fs::OFlags::NOFOLLOW).bits() as i32,
            );
        }
        root.open_with(path.as_str(), &options).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BrokerError::NotFound(path.clone())
            } else {
                BrokerError::io("open Vault object", error)
            }
        })
    }

    pub(crate) fn validate_file_metadata(
        path: &VaultPath,
        metadata: &cap_std::fs::Metadata,
    ) -> Result<(), BrokerError> {
        if !metadata.is_file() {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "not a regular file",
            });
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            return Err(BrokerError::UnsafeNode {
                path: path.clone(),
                reason: "hard-linked or unlinked file",
            });
        }
        Ok(())
    }
}
