//! 维护观测只包含固定逻辑元数据；原请求主体不会被 OS 操作者替换。
use crate::{
    BrokerError, ContentAuditPhase, ReconciliationOutcome, Revision, VaultPath, VaultUuid,
    WriterStatus,
};
use serde::{Deserialize, Serialize};

/// 实际本机 OS 操作者；应用会话或配置 owner 不能替代它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceActor {
    /// 非 setuid/setgid 的实际进程主体。
    LocalOs {
        /// 实际 UID。
        uid: u32,
        /// 实际 GID。
        gid: u32,
    },
}
impl MaintenanceActor {
    /// 从内核取得真实身份，并拒绝真实与有效身份不一致的进程。
    pub fn current() -> Result<Self, BrokerError> {
        use rustix::process::{getegid, geteuid, getgid, getuid};
        if getuid() != geteuid() || getgid() != getegid() {
            return Err(BrokerError::InvalidArtifact("不允许提权身份维护".into()));
        }
        Ok(Self::LocalOs {
            uid: getuid().as_raw(),
            gid: getgid().as_raw(),
        })
    }
}

/// 已审阅目标允许的封闭维护动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceDecision {
    /// 取消原请求尚未安装的准备态。
    CancelPrepared,
    /// 保留当前文件并封存三方证据，原请求永久未知。
    PreserveInstalling,
    /// 取消尚未开始安装的恢复意图。
    CancelRestoreIntent,
}
/// 原请求的可维护阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceTargetKind {
    /// 安装尚未开始。
    Prepared,
    /// 安装曾开始，结果不能猜测。
    Installing,
    /// 只持久化恢复意图，尚未开始安装。
    RestoreIntent,
}
/// 脱敏的固定不可观测原因；不含底层路径或错误正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceUnavailableReason {
    /// 当前节点不安全或在观察期间变化。
    UnsafeOrUnstable,
    /// 观察超过固定内部预算。
    LimitExceeded,
    /// 固定根仍可信，但目标 I/O 暂不可用。
    IoUnavailable,
}
/// 审阅的当前文件证据，未观测不能冒充缺失。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceCurrent {
    /// 此类审阅不读取当前正文。
    NotObserved,
    /// 未能可靠观察，绝不是 Missing。
    Unobservable {
        /// 固定脱敏原因。
        reason: MaintenanceUnavailableReason,
    },
    /// 审阅时路径不存在。
    Missing,
    /// 审阅时路径的完整内容摘要。
    Present {
        /// 与 Vault 和逻辑路径绑定的 revision。
        revision: Revision,
    },
}
/// 一次有界审阅的逻辑结果，不包含原请求键或正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceTarget {
    /// 绑定阶段及文件证据的 64 位十六进制选择器。
    pub review_id: String,
    /// 原请求固定指纹，不能作为新的 mutation 授权。
    pub request_fingerprint: String,
    /// 原目标逻辑路径。
    pub path: VaultPath,
    /// 原请求阶段。
    pub kind: MaintenanceTargetKind,
    /// 原操作序号，尚未准备安装的恢复为 None。
    pub sequence: Option<u64>,
    /// 原请求的 CAS 基线。
    pub previous_revision: Option<Revision>,
    /// 原拟写正文 revision，不证明曾安装。
    pub target_revision: Option<Revision>,
    /// typed 文件操作选择器；旧事件完全省略此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_operation_id: Option<String>,
    /// 审阅时的当前文件证据。
    pub current: MaintenanceCurrent,
    /// 完整提案可用性；Rename Prepared 为准备时事实，执行态为本次保全观察。
    pub proposed_available: bool,
    /// Rename 的第二路径审阅；旧维护事件完全省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<super::MaintenanceSecondaryTarget>,
    /// Rename 原动作、阶段及已知子效果；不是执行或身份许可。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_operation: Option<super::MaintenanceFileOperation>,
}
/// 维护本身的结果，与原请求结果严格区分。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceOutcome {
    /// 原请求已按本次明确决定形成持久终态。
    Applied,
    /// 只取消了尚未执行的维护，原请求保持原样。
    Cancelled,
    /// 执行前发现审阅变化，本次没有协调原请求。
    NotApplied,
}
/// 固定 writer.maintenance 事件，意图与结果共用事件 ID。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceAuditEvent {
    /// 旧协议省略；Delete/Restore 为 Some(2)，Rename 为 Some(3)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    /// 固定事件 ID，64 位小写十六进制。
    pub event_id: String,
    /// 意图或持久结果。
    pub phase: ContentAuditPhase,
    /// 配对的 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 实际执行维护的 OS 主体，不覆盖原请求主体。
    pub actor: MaintenanceActor,
    /// 首次固定的维护动作。
    pub decision: MaintenanceDecision,
    /// 首次请求时间；无法取得则为 unknown。
    pub requested_at_ms: Option<i64>,
    /// 首次固定的完整逻辑审阅。
    pub target: MaintenanceTarget,
    /// 维护结果；Intent 必须为 None。
    pub outcome: Option<MaintenanceOutcome>,
    /// 原请求的协调结果；只有 Applied 才有值。
    pub resolution: Option<ReconciliationOutcome>,
    /// 原未知请求的私有证据包 ID。
    pub evidence_id: Option<String>,
}
/// 维护结果尚须取得实际 Outcome 审计 ACK，不能单凭本值开放写入。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenanceReceipt {
    /// 固定维护 Outcome。
    pub event: MaintenanceAuditEvent,
    /// 原有 canonical 提交水位；维护不提高它。
    pub committed_through: u64,
}
/// 唯一维护槽的当前状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenancePending {
    /// 维护槽选择器，与原请求 review_id 不同。
    pub maintenance_id: String,
    /// 固定原始维护意图。
    pub intent: MaintenanceAuditEvent,
    /// 是否已持久核对真实 Intent 审计回执。
    pub intent_confirmed: bool,
    /// 是否曾开始调用原请求协调，开始后不能取消维护自身。
    pub execution_started: bool,
    /// 已知持久维护结果，未知时为 None。
    pub outcome: Option<MaintenanceAuditEvent>,
    /// 是否已持久核对真实 Outcome 审计回执。
    pub outcome_confirmed: bool,
}
/// 有界维护审阅，不读取或输出 proposed 正文。
#[derive(Clone, Debug, Serialize)]
pub struct MaintenanceStatus {
    /// 写者真实阶段及门禁。
    pub writer: WriterStatus,
    /// 最多 64 项的可审阅原请求。
    pub targets: Vec<MaintenanceTarget>,
    /// 尚未取得 Outcome ACK 的唯一维护槽。
    pub pending: Option<MaintenancePending>,
}
