//! 只持久化固定正文事件；owner 完成权威 AuditLedger 的完整回执检查后确认。

pub(crate) mod delivery;
mod event;
mod status;
pub(crate) mod store;
pub(crate) use store::{stage_intent, stage_outcome};

use crate::{MutationActor, MutationKind, Revision, VaultPath, VaultUuid};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 同一固定正文操作的审计阶段，事件 ID 与阶段共同标识一条记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAuditPhase {
    /// 首次持久化的写入意图，不证明安装已经发生。
    Intent,
    /// 从永久操作终态派生的结果，不接受临时 HTTP 错误充当结果。
    Outcome,
}

/// 永久操作终态；临时 HTTP 故障不能据此制造 Indeterminate。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAuditOutcome {
    /// Broker 已持久确认完成原操作。
    Committed,
    /// CAS 不匹配，原请求已形成不可复活的拒绝终态。
    Rejected,
    /// 尚未安装的原请求已被明确取消。
    Cancelled,
    /// 原安装结果未知，受信协调已封存证据，不能冒认成功或失败。
    Indeterminate,
}

/// 不包含正文、凭证、任意 JSON 或任意 action；时间首次持久化后不再改写。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentAuditEvent {
    /// 首次请求固定的关联 ID；Intent 与 Outcome 共用此 ID。
    pub event_id: String,
    /// 此记录是操作意图还是持久终态结果。
    pub phase: ContentAuditPhase,
    /// 原操作绑定的 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 原操作绑定的逻辑路径，不是主机绝对路径。
    pub path: VaultPath,
    /// 原操作的固定保存或恢复语义，重放时不能更换。
    pub kind: MutationKind,
    /// 由可信 owner 提供并在首次请求中固定的主体或摄入来源。
    pub actor: MutationActor,
    /// 首次请求持久化时的服务器 Unix 毫秒时间；无法取得时为 None。
    pub requested_at_ms: Option<i64>,
    /// 原请求要求匹配的基线；create 的不存在前置条件表示为 None。
    pub previous_revision: Option<Revision>,
    /// 拟写正文的路径绑定 revision；仅 Committed Outcome 证明它曾被提交。
    pub new_revision: Revision,
    /// 拟写正文的 SHA-256 摘要，不包含正文。
    pub sha256: String,
    /// 拟写正文的字节数。
    pub bytes: u64,
    /// 永久结果；Intent 为 None，Outcome 必须有值。
    pub outcome: Option<ContentAuditOutcome>,
}

/// 不透明候选不保活 Broker 租约，也不能构造、反序列化或跨实例确认。
pub struct ContentAuditCandidate {
    pub(crate) instance: Arc<()>,
    pub(crate) record_id: String,
    pub(crate) event: ContentAuditEvent,
    pub(crate) digest: String,
}

impl ContentAuditCandidate {
    /// owner 用固定映射构造 AuditEvent，核对完整返回记录后才能确认本候选。
    pub fn event(&self) -> &ContentAuditEvent {
        &self.event
    }
}

/// 有界持久待办状态；已明确未安装的拒绝和取消不会冻结普通编辑。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentAuditStatus {
    /// 尚未取得审计确认的事件数量，按事件而非请求计数。
    pub pending: usize,
    /// 存在未结案的恢复，或恢复的已提交/未知结果尚未审计确认；明确拒绝与取消不计入。
    pub restore_unconfirmed: bool,
}
