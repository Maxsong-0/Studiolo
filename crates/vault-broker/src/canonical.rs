//! 对既有 canonical 根的身份与有界内容访问，不具备写权限。

use std::path::{Path, PathBuf};
use studiolo_core::{Revision, VaultPath, VaultUuid};

use crate::{BrokerError, ReadOnlyVault, VaultIdentity};

/// 一次稳定的 canonical 读取；不把索引代次当作编辑 revision。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalRead {
    /// 从既有 canonical 标记读出的持久身份。
    pub vault_uuid: VaultUuid,
    /// 已批准 canonical 根之下的逻辑路径。
    pub path: VaultPath,
    /// 精确字节；文本编码与渲染由调用方负责。
    pub content: Vec<u8>,
    /// 沿用 Broker 既有 revision 算法，绑定 Vault、路径与精确字节。
    pub revision: Revision,
}

/// 针对一个已批准、已存在 canonical 根的窄读取句柄。
///
/// 该句柄不创建标记、数据目录、历史、快照或缓存，也没有变更能力。
/// 每次读取前后都会复核选定根、有界身份记录与批量屏障。
/// 这些检查能发现已被观察到的替换，但不会阻止无关的文件系统写者。
pub struct CanonicalReadVault {
    reader: ReadOnlyVault,
    selected_root: PathBuf,
    vault_uuid: VaultUuid,
}

impl CanonicalReadVault {
    /// 打开由可信组合根批准的既有 canonical 根。
    ///
    /// 要求绝对路径，并已存在 `format_version` 为 1、不超过 4096 字节的 `.workbench/vault.json`。
    /// 缺失或非法的状态绝不会被初始化或修复。调用方不得从 HTTP 请求取得此路径。
    pub fn open_existing(root: &Path) -> Result<Self, BrokerError> {
        if !root.is_absolute() {
            return Err(BrokerError::io(
                "open canonical root",
                std::io::Error::other("an approved absolute root is required"),
            ));
        }
        let reader = ReadOnlyVault::open(root)?;
        let vault_uuid = identity(&reader)?;
        let handle = Self {
            reader,
            selected_root: root.to_owned(),
            vault_uuid,
        };
        handle.check_binding()?;
        Ok(handle)
    }

    /// 返回已固定的 Vault 持久身份，并不断言其新鲜度。
    #[must_use]
    pub fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    /// 返回仍然绑定的目录身份，供可信启动组合核对。
    /// 该值不用于 HTTP 响应，也不用于调用方认证。
    #[cfg(unix)]
    pub fn root_identity(&self) -> Result<(u64, u64), BrokerError> {
        self.check_binding()?;
        self.reader.root_identity()
    }

    /// 在调用方给定的字节上限内读取可见的普通文件。
    ///
    /// 调用方必须施加其端点自身的上限；上限为零时只允许空文件。
    /// 超过上限的增长会被拒绝，内存中最多只保留上限加一字节的内容。
    /// 隐藏路径、链接与特殊文件一律拒绝。
    /// 单文件内容稳定并不代表派生索引是新鲜的。
    pub fn read_file(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, BrokerError> {
        self.check_binding()?;
        let note = self
            .reader
            .read_in_namespace(path, max_bytes, self.vault_uuid)?;
        self.check_binding()?;
        Ok(CanonicalRead {
            vault_uuid: self.vault_uuid,
            path: note.path,
            content: note.content,
            revision: note.revision,
        })
    }

    fn check_binding(&self) -> Result<(), BrokerError> {
        self.check_root()?;
        self.reader.check_batch_barrier()?;
        if identity(&self.reader)? != self.vault_uuid {
            return Err(BrokerError::CorruptIdentity);
        }
        self.check_root()
    }

    #[cfg(unix)]
    fn check_root(&self) -> Result<(), BrokerError> {
        let current =
            ReadOnlyVault::open(&self.selected_root).map_err(|_| BrokerError::RootChanged)?;
        if current
            .root_identity()
            .map_err(|_| BrokerError::RootChanged)?
            != self
                .reader
                .root_identity()
                .map_err(|_| BrokerError::RootChanged)?
        {
            return Err(BrokerError::RootChanged);
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn check_root(&self) -> Result<(), BrokerError> {
        Err(BrokerError::RootChanged)
    }
}

fn identity(reader: &ReadOnlyVault) -> Result<VaultUuid, BrokerError> {
    let bytes = reader.read_identity().map_err(|error| match error {
        BrokerError::NotFound(_)
        | BrokerError::ReadLimitExceeded { .. }
        | BrokerError::UnsafeNode { .. } => BrokerError::CorruptIdentity,
        other => other,
    })?;
    VaultIdentity::decode(&bytes).map_err(|_| BrokerError::CorruptIdentity)
}
