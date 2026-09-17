//! 类型化文件效果：Missing 与空文件明确区分，元数据只绑定请求，不充当认证。
use crate::{BrokerError, MutationActor, Revision, VaultPath};
use serde::{Deserialize, Serialize};

/// 由可信 owner 生成的范围摘要；客户端输入本身不能取得授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FileOperationScopeHash(String);
impl FileOperationScopeHash {
    /// 只接受 64 个小写十六进制字符。
    pub fn new(value: impl Into<String>) -> Result<Self, BrokerError> {
        let value = value.into();
        if !crate::writer::disk::hex(&value) {
            return Err(BrokerError::IdempotencyConflict);
        }
        Ok(Self(value))
    }
    /// 固定请求绑定的摘要，不是会话凭据。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for FileOperationScopeHash {
    type Error = BrokerError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<FileOperationScopeHash> for String {
    fn from(value: FileOperationScopeHash) -> Self {
        value.0
    }
}

/// 单文件封闭请求；没有覆盖、目录递归删除或永久删除入口。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOperationRequest {
    /// 单文件关联复制后删除；目标固定为不存在，不提供覆盖能力。
    Rename {
        /// 已批准的源逻辑路径。
        from: VaultPath,
        /// 源路径绑定的精确 CAS 版本。
        expected: Revision,
        /// 已批准且不存在的目标逻辑路径。
        to: VaultPath,
        /// 原请求重放键；不生成独立子步骤权限。
        idempotency_key: String,
    },
    /// 删除已批准的普通文件；缺失也必须形成精确 CAS 拒绝。
    Delete {
        /// 原逻辑路径。
        path: VaultPath,
        /// 路径绑定的精确旧版本。
        expected: Revision,
        /// 原始键仅在调用内使用，持久记录只保存摘要。
        idempotency_key: String,
    },
    /// 只恢复同一墓碑的原路径，目标固定 MustNotExist。
    RestoreTombstone {
        /// 必须与墓碑原路径一致。
        path: VaultPath,
        /// 服务端列表给出的固定墓碑选择器。
        tombstone_id: String,
        /// 新操作的幂等键，不复用原删除请求。
        idempotency_key: String,
    },
}

/// 实际主体与当前范围的绑定；构造此值不会验证身份或授予文件权限。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOperationMetadata {
    pub(crate) actor: MutationActor,
    pub(crate) scope_hash: FileOperationScopeHash,
    pub(crate) scope_epoch: u64,
}
impl FileOperationMetadata {
    /// 本 API 仅用于可信 owner 批准的 Full 保全对象，不接 raw 到期生命周期。
    pub fn new(
        actor: MutationActor,
        scope_hash: FileOperationScopeHash,
        scope_epoch: u64,
    ) -> Result<Self, BrokerError> {
        crate::MutationMetadata::new(actor.clone(), crate::MutationKind::Replace)?;
        if scope_epoch == 0 {
            return Err(BrokerError::IdempotencyConflict);
        }
        Ok(Self {
            actor,
            scope_hash,
            scope_epoch,
        })
    }
}

/// 精确执行证据；零字节文件仍是 Present，不能与 Missing 合并。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FilePresence {
    /// 协议在记录效果时确认路径缺失。
    Missing,
    /// 协议在记录效果时完整核验的普通文件。
    Present {
        /// 与 Vault 和路径绑定的版本。
        revision: Revision,
        /// 原始字节摘要。
        sha256: String,
        /// 精确逻辑字节数。
        bytes: u64,
    },
}

/// 持久子效果不代表此刻路径仍处于相同状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileEffect {
    /// 已有完整执行证据的前后状态。
    Confirmed {
        /// 被改变的路径。
        path: VaultPath,
        /// 执行时的旧状态。
        before: FilePresence,
        /// 执行时确认的新状态。
        after: FilePresence,
    },
    /// 本次未进入可见变更；不声明路径现在存在或缺失。
    NotApplied {
        /// 仅标识被请求的路径。
        path: VaultPath,
    },
    /// 曾发出可见变更，现有证据不足以重建成功。
    Unknown {
        /// 结果未知的路径。
        path: VaultPath,
    },
}

/// 原请求永久结果，与审计 ACK 及维护自身结果分开。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationOutcome {
    /// 已有完整持久执行效果。
    Committed,
    /// 已知 CAS 不匹配，未执行。
    Rejected,
    /// 在可见执行前明确取消。
    Cancelled,
    /// 经过受信保全，原请求永久无法确认为完成。
    Indeterminate,
}

/// 已知持久终态；装配必须另外核对 audit_pending 才能向用户宣布完成。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOperationReceipt {
    /// 固定操作选择器。
    pub operation_id: String,
    /// 同一 writer 的分配序号；非成功不推进提交水位。
    pub sequence: u64,
    /// 原请求的永久结果。
    pub outcome: FileOperationOutcome,
    /// 有序效果；本片只有一个路径，后续复合操作最多两个。
    pub effects: Vec<FileEffect>,
    /// 成功删除生成的墓碑；恢复不消费原墓碑。
    pub tombstone_id: Option<String>,
    /// 受信保全产生的证据包。
    pub evidence_id: Option<String>,
}
