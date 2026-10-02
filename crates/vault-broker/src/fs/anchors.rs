//! 同步目录锚点的有根预检与显式建立。
use crate::{BrokerError, FsVaultBroker};
use cap_std::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use studiolo_core::VaultPath;

impl FsVaultBroker {
    /// 完整目录路径，不借文件名表示锚点。只读时返回缺失，创建时返回末目录是否新建。
    pub(crate) fn sync_anchor_directory(
        &self,
        path: &VaultPath,
        create: bool,
    ) -> Result<bool, BrokerError> {
        self.validate_name_lengths(path)?;
        let committed = self.sync_anchors_ready()?;
        let root = self
            .root
            .dir_metadata()
            .map_err(|e| BrokerError::io("anchor root", e))?;
        let unsafe_node = |reason| BrokerError::UnsafeNode {
            path: path.clone(),
            reason,
        };
        let mut current = PathBuf::new();
        let mut expected_parent = (root.dev(), root.ino());
        let mut created = false;
        for component in path.as_str().split('/') {
            let parent = if current.as_os_str().is_empty() {
                Path::new(".")
            } else {
                &current
            };
            let directory = Self::open_directory(&self.root, parent, path)?;
            let identity = directory
                .dir_metadata()
                .map_err(|e| BrokerError::io("anchor parent", e))?;
            if (identity.dev(), identity.ino()) != expected_parent {
                return Err(BrokerError::CaptureChanged);
            }
            if identity.dev() != root.dev() {
                return Err(unsafe_node("cross-device sync anchor"));
            }
            for entry in directory
                .entries()
                .map_err(|e| BrokerError::io("list anchor parent", e))?
            {
                let entry = entry.map_err(|e| BrokerError::io("read anchor parent", e))?;
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| unsafe_node("non-UTF-8 name"))?;
                if name != component
                    && anchor_component_key(name) == anchor_component_key(component)
                {
                    return Err(BrokerError::PathCollision {
                        requested: path.clone(),
                        existing: current.join(name).to_string_lossy().into_owned(),
                    });
                }
            }
            created = false;
            let metadata = match directory.symlink_metadata(component) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(true);
                    }
                    if self.sync_anchors_ready()? != committed {
                        return Err(BrokerError::CaptureChanged);
                    }
                    let reopened = Self::open_directory(&self.root, parent, path)?;
                    let same = reopened
                        .dir_metadata()
                        .map_err(|e| BrokerError::io("anchor parent", e))?;
                    if (same.dev(), same.ino()) != (identity.dev(), identity.ino()) {
                        return Err(BrokerError::CaptureChanged);
                    }
                    match rustix::fs::mkdirat(
                        &directory,
                        component,
                        rustix::fs::Mode::from_raw_mode(0o700),
                    ) {
                        Ok(()) => {
                            created = true;
                        }
                        Err(error) if error == rustix::io::Errno::EXIST => {}
                        Err(error) => {
                            return Err(BrokerError::io("create sync anchor", error.into()));
                        }
                    }
                    directory
                        .symlink_metadata(component)
                        .map_err(|e| BrokerError::io("new anchor", e))?
                }
                Err(error) => return Err(BrokerError::io("inspect sync anchor", error)),
            };
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.dev() != root.dev()
            {
                return Err(unsafe_node(
                    "linked, non-directory or cross-device sync anchor",
                ));
            }
            if create {
                // 重跑也落盘已有分量，涵盖上次 mkdir 成功而父目录 fsync 失败的情况。
                Self::sync_directory(&directory, Path::new("."))?;
            }
            current.push(component);
            expected_parent = (metadata.dev(), metadata.ino());
            let reopened = Self::open_directory(&self.root, &current, path)?;
            let same = reopened
                .dir_metadata()
                .map_err(|e| BrokerError::io("anchor child", e))?;
            if (same.dev(), same.ino()) != (metadata.dev(), metadata.ino())
                || self.sync_anchors_ready()? != committed
            {
                return Err(BrokerError::CaptureChanged);
            }
        }
        // 创建期间的名字碰撞或替换不能给后续锚点提供成功证据。
        if create && self.sync_anchor_directory(path, false)? {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(created)
    }
}

fn anchor_component_key(component: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    component
        .nfc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}
