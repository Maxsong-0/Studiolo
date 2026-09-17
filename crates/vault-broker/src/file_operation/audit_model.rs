//! 文件操作固定审计事件；缺失和恢复不套旧 ContentAuditEvent 的必有新版本字段。
use super::model::{FileEffect, FileOperationOutcome, FileOperationScopeHash};
use crate::{ContentAuditPhase, MutationActor, Revision, VaultPath, VaultUuid};
use serde::{Deserialize, Serialize};

/// 固定动作子类；恢复由装配投递到 recovery.page_restore。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileOperationKind {
    /// vault.mutation 的复合重命名子类；事件 path 为源。
    Rename {
        /// 源的精确 CAS 条件。
        expected: Revision,
        /// 固定为 MustNotExist 的目标逻辑路径。
        to: VaultPath,
    },
    /// vault.mutation 的删除子类。
    Delete {
        /// 删除前的精确 CAS 条件。
        expected: Revision,
    },
    /// 最近删除恢复；目标条件固定为原路径缺失。
    RestoreTombstone {
        /// 被选中的墓碑，不会在恢复成功后被消费。
        tombstone_id: String,
    },
}

/// 完整固定事件；offset/hash ACK 只代表可信 owner 已核验真实账本，事件本身不是授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileOperationAuditEvent {
    /// 本审计 payload 的显式版本。
    pub schema_version: u32,
    /// 意图与结果共享的稳定事件身份。
    pub event_id: String,
    /// 固定请求选择器。
    pub operation_id: String,
    /// 意图或持久结果。
    pub phase: ContentAuditPhase,
    /// 逻辑 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 原实际主体，不由维护操作者替换。
    pub actor: MutationActor,
    /// 固定逻辑路径。
    pub path: VaultPath,
    /// 固定动作与条件。
    pub kind: FileOperationKind,
    /// 请求绑定的范围摘要，不充当身份或许可。
    pub scope_hash: FileOperationScopeHash,
    /// 请求绑定的范围代次。
    pub scope_epoch: u64,
    /// 首次冻结的服务器时间；没有可信值则 unknown。
    pub requested_at_ms: Option<i64>,
    /// 完整请求指纹，不含原始幂等键或正文。
    pub request_fingerprint: String,
    /// 仅 Outcome 才有永久结果。
    pub outcome: Option<FileOperationOutcome>,
    /// Intent 为空；Outcome 仅携带持久效果。
    pub effects: Vec<FileEffect>,
    /// 删除成功产生的固定墓碑。
    pub tombstone_id: Option<String>,
    /// 原未知操作的保全包。
    pub evidence_id: Option<String>,
}
