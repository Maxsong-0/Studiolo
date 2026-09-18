//! 固定 action `sync.transport.run` 的封闭候选；payload 不放 stdout、token 或原配置。
//!
//! 本 crate 只给封闭候选与已核验回执的登记位；真实 AuditRecord 由可信装配追加并核验。

use super::approval::{RunBudget, TransportBindingFacts};
use super::state::RunRecord;
use super::tickets::TicketBinding;
use super::types::{Quiescence, ResourceCounts, RunId, RunOperation, StopReason};
use sha2::{Digest, Sha256};

/// 固定敏感动作名。
pub const TRANSPORT_RUN_ACTION: &str = "sync.transport.run";

/// 两个 phase；一个活动槽最多两个待补事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditPhaseKind {
    /// 启动前的固定意图。
    Intent,
    /// 闭合时的固定结果。
    Outcome,
}

impl AuditPhaseKind {
    /// 事件标识后缀；与 run/phase 稳定绑定。
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Outcome => "outcome",
        }
    }
}

/// 请求主体来源；第一片只有 owner 自己的会话。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequesterSource {
    /// 已鉴权的主人会话。
    OwnerSession,
}

/// 闭合摘要；只报告「是否实际观察到退出」，不报告成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutcomeSummary {
    /// 是否真的创建过受控容器。
    pub container_created: bool,
    /// 已持久的停止原因。
    pub stop_reason: Option<StopReason>,
    /// 是否实际取得进程等待结果。
    pub exit_observed: bool,
    /// 静止是否已被证明。
    pub quiescence: Quiescence,
    /// 实测资源计数。
    pub counts: ResourceCounts,
}

/// 固定审计 payload；字段闭合，没有任意 JSON 入口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditPayload {
    /// 固定为 1。
    pub schema_version: u8,
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号。
    pub sequence: u64,
    /// 固定操作。
    pub operation: RunOperation,
    /// 绑定摘要；不含原配置正文。
    pub binding_digest: [u8; 32],
    /// 运行时摘要。
    pub runtime_digest: [u8; 32],
    /// 后端摘要。
    pub backend_digest: [u8; 32],
    /// 运行预算。
    pub budget: RunBudget,
    /// 请求主体来源。
    pub requester: RequesterSource,
    /// 仅 Outcome 阶段存在。
    pub outcome: Option<OutcomeSummary>,
}

/// 待补审计候选；私有字段，不可构造也不可反序列化。
///
/// ```compile_fail
/// let candidate: studiolo_sync_transport::owned_run::RunAuditCandidate =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct RunAuditCandidate {
    pub(crate) binding: TicketBinding,
    pub(crate) phase: AuditPhaseKind,
    pub(crate) event_id: String,
    pub(crate) payload: AuditPayload,
}

impl RunAuditCandidate {
    /// 固定 action 名。
    #[must_use]
    pub fn action(&self) -> &'static str {
        TRANSPORT_RUN_ACTION
    }

    /// Intent 或 Outcome。
    #[must_use]
    pub fn phase(&self) -> AuditPhaseKind {
        self.phase
    }

    /// 与 run/phase 稳定绑定的事件标识。
    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    /// 固定 payload。
    #[must_use]
    pub fn payload(&self) -> &AuditPayload {
        &self.payload
    }
}

/// 生成与 run/phase 稳定绑定的事件标识。
#[must_use]
pub fn event_id(run_id: RunId, phase: AuditPhaseKind) -> String {
    format!("{TRANSPORT_RUN_ACTION}/{run_id}/{}", phase.suffix())
}

/// 绑定摘要；配置正文、凭证与远端标识都不进审计。
#[must_use]
pub fn binding_digest(binding: &TransportBindingFacts) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-transport.binding.v1\0");
    hash.update(binding.vault_uuid.to_string().as_bytes());
    hash.update(binding.link_id);
    hash.update(binding.remote_vault);
    hash.update(binding.scope_epoch.get().to_be_bytes());
    hash.update(binding.scope_hash);
    hash.update(binding.approved_config_digest);
    hash.finalize().into()
}

/// 从活动记录生成闭合摘要。
#[must_use]
pub fn outcome_summary(record: &RunRecord) -> OutcomeSummary {
    OutcomeSummary {
        container_created: record.container.is_some(),
        stop_reason: record.stop_reason,
        exit_observed: record.exit.reaped(),
        quiescence: record.observation().quiescence,
        counts: record.counts,
    }
}
