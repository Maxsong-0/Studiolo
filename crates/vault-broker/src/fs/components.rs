//! 逻辑路径逐分量校验（别名、链接与特殊节点），以及父目录的有根创建。

use std::path::{Path, PathBuf};

use cap_std::fs::MetadataExt as _;
use studiolo_core::VaultPath;

use crate::{BrokerError, FsVaultBroker};

impl FsVaultBroker {
    pub(crate) fn validate_components(
        &self,
        path: &VaultPath,
        allow_missing: bool,
    ) -> Result<(), BrokerError> {
        let components: Vec<&str> = path.as_str().split('/').collect();
        let mut current = PathBuf::new();
        for (index, component) in components.iter().enumerate() {
            let parent = current.clone();
            let requested_key = collision_component_key(component);
            let parent_path = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &parent
            };
            let parent_dir = Self::open_directory(&self.root, parent_path, path)?;
            let listing = parent_dir.entries();
            match listing {
                Ok(entries) => {
                    for entry in entries {
                        let entry =
                            entry.map_err(|error| BrokerError::io("read directory", error))?;
                        let existing = entry.file_name();
                        let existing =
                            existing.to_str().ok_or_else(|| BrokerError::UnsafeNode {
                                path: path.clone(),
                                reason: "non-UTF-8 name",
                            })?;
                        if existing != *component
                            && collision_component_key(existing) == requested_key
                        {
                            let existing_path = parent.join(existing);
                            return Err(BrokerError::PathCollision {
                                requested: path.clone(),
                                existing: existing_path.to_string_lossy().into_owned(),
                            });
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(BrokerError::NotFound(path.clone()));
                }
                Err(error) => return Err(BrokerError::io("list path parent", error)),
            }
            current.push(component);
            match parent_dir.symlink_metadata(component) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "symbolic link",
                        });
                    }
                    let is_last = index + 1 == components.len();
                    if !is_last && !metadata.is_dir() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "non-directory path component",
                        });
                    }
                    if is_last && !metadata.is_file() {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "not a regular file",
                        });
                    }
                    #[cfg(unix)]
                    if is_last && metadata.nlink() > 1 {
                        return Err(BrokerError::UnsafeNode {
                            path: path.clone(),
                            reason: "hard-linked file",
                        });
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(BrokerError::NotFound(path.clone()));
                }
                Err(error) => return Err(BrokerError::io("inspect Vault path", error)),
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_parent_dirs(&self, path: &VaultPath) -> Result<(), BrokerError> {
        let mut current = PathBuf::new();
        let mut components = path.as_str().split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            let parent_path = if current.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &current
            };
            let parent_dir = Self::open_directory(&self.root, parent_path, path)?;
            match parent_dir.symlink_metadata(component) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(BrokerError::UnsafeNode {
                        path: path.clone(),
                        reason: "unsafe parent directory",
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match parent_dir.create_dir(component) {
                        Ok(()) => Self::sync_directory(&parent_dir, Path::new("."))?,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(BrokerError::io("create Vault directory", error)),
                    }
                }
                Err(error) => return Err(BrokerError::io("inspect Vault directory", error)),
            }
            current.push(component);
            // 创建后从已批准的根重新打开：并发的改名或链接替换，
            // 不得成为后续子目录的权威来源。
            Self::open_directory(&self.root, &current, path)?;
        }
        Ok(())
    }
}

/// 磁盘条目名先归一到 NFC 再折叠大小写：VaultPath 只接受 NFC，而外部写者可能留下 NFD 拼写，
/// 在归一化不敏感的卷上两者是同一物理文件。
pub(crate) fn collision_component_key(component: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    component.nfc().collect::<String>().to_lowercase()
}
