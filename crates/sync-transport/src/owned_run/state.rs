//! 持久 schema v1：单槽单 writer 的完整记录与失败关闭解析。
//!
//! 解析永远只接受「旧版或新版的完整记录」：未知版本、半 JSON、未知字段、
//! 自相矛盾的阶段字段一律拒绝，绝不做部分修复或自动迁移。

use super::approval::{BackendDescriptor, RootPins, RunBudget, RuntimePins, TransportBindingFacts};
use super::error::{Result, TransportError};
use super::types::{
    ContainerIdentity, ExitObservation, Quiescence, ResourceCounts, RunDiagnostics, RunId,
    RunObservation, RunOperation, RunPhase, StopReason, TerminalSummary,
};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

mod consistency;
use consistency::record_consistent;

/// 固定 schema 版本；后续版本不得回写降级。
pub const SCHEMA_VERSION: u8 = 1;
/// 单个 state 的字节硬上限；不存日志正文。
pub const STATE_MAX_BYTES: usize = 64 * 1024;

/// 单槽持久状态；完整原子替换，永不部分写入。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportState {
    /// 固定为 1。
    pub schema_version: u8,
    /// 显式建库时生成的随机安装标识。
    pub installation_id: [u8; 16],
    /// 单调提交代次。
    pub generation: u64,
    /// 下一个待分配序号；已分配序号永不复用。
    pub next_sequence: u64,
    /// 请求绑定。
    pub binding: TransportBindingFacts,
    /// 根的规范路径与对象身份。
    pub roots: RootPins,
    /// 固定运行时。
    pub runtime: RuntimePins,
    /// 受控后端描述。
    pub backend: BackendDescriptor,
    /// 运行 UID/GID。
    pub owner: OwnerIdentity,
    /// 至多一个活动运行；不因进程句柄丢失而清空。
    pub active: Option<RunRecord>,
    /// 至多一个已审计闭合摘要。
    pub last_terminal: Option<TerminalSummary>,
}

/// 运行身份；与 payload 专用身份必须不同。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerIdentity {
    /// 真实运行 UID。
    pub uid: u32,
    /// 真实运行 GID。
    pub gid: u32,
}

/// 静止证明摘要；只记录能真正支撑该结论的证据种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuiescenceEvidence {
    /// 同一受控 payload 子组实测 `cgroup.events populated=0`。
    PayloadCgroupUnpopulated,
    /// boot 已改变，旧 boot 的进程必然不存在；退出结果仍然未知。
    BootChanged,
}

/// 静止证明；缺少「入口已关闭」就不是证明。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuiescenceProof {
    /// 证据种类。
    pub evidence: QuiescenceEvidence,
    /// 取证时的 payload 子组身份；只有 `BootChanged` 且当时没有登记容器才允许缺省。
    pub cgroup_identity: Option<(u64, u64)>,
    /// 继续创建 payload 的路径是否已关闭。
    pub spawn_gate_closed: bool,
}

/// 已由 owner 核验过的真实 AuditRecord 回执；任意 offset/hash 本身不是认证。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditReceipt {
    /// 账本偏移，非零。
    pub offset: NonZeroU64,
    /// 记录哈希。
    pub hash: [u8; 32],
}

/// 一个审计槽：固定事件标识加可选回执。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditSlot {
    /// 与 run/phase 稳定绑定的事件标识。
    pub event_id: String,
    /// 已确认的回执；`None` 表示待补。
    pub receipt: Option<AuditReceipt>,
}

impl AuditSlot {
    /// 是否仍待补。
    #[must_use]
    pub fn pending(&self) -> bool {
        self.receipt.is_none()
    }
}

/// 活动运行记录；固定字段，不放 stdout、token 或原配置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    /// 运行标识。
    pub run_id: RunId,
    /// 运行序号。
    pub sequence: u64,
    /// 固定操作。
    pub operation: RunOperation,
    /// 请求时刻（调用方提供的毫秒数）。
    pub requested_at_millis: u64,
    /// 运行预算。
    pub budget: RunBudget,
    /// 当前阶段。
    pub phase: RunPhase,
    /// 单调停止原因。
    pub stop_reason: Option<StopReason>,
    /// 请求启动 launcher 时的 boot 标识；重开后用来判断是否已跨 boot。
    pub requested_boot_id: Option<[u8; 16]>,
    /// 受控容器身份。
    pub container: Option<ContainerIdentity>,
    /// 仅诊断的 PID/start-time。
    pub diagnostics: RunDiagnostics,
    /// 原始退出结果，可未知。
    pub exit: ExitObservation,
    /// 静止证明摘要。
    pub quiescence: Option<QuiescenceProof>,
    /// 实测资源计数。
    pub counts: ResourceCounts,
    /// Intent 审计槽。
    pub intent: AuditSlot,
    /// Outcome 审计槽；固定 Outcome 保存后才出现。
    pub outcome: Option<AuditSlot>,
}

impl RunRecord {
    /// 四项分开报告的当前观察。
    #[must_use]
    pub fn observation(&self) -> RunObservation {
        RunObservation {
            phase: self.phase,
            quiescence: match self.quiescence {
                Some(proof) if proof.spawn_gate_closed => Quiescence::Confirmed,
                _ => Quiescence::Unconfirmed,
            },
            exit: self.exit,
            stop_reason: self.stop_reason,
        }
    }

    /// 是否还有待补审计事件。
    #[must_use]
    pub fn audit_pending(&self) -> bool {
        self.intent.pending() || self.outcome.as_ref().is_some_and(AuditSlot::pending)
    }

    /// 已闭合摘要；只在 Outcome 已 ACK 后调用。
    #[must_use]
    pub fn terminal(&self) -> TerminalSummary {
        TerminalSummary {
            run_id: self.run_id,
            sequence: self.sequence,
            operation: self.operation,
            observation: self.observation(),
            counts: self.counts,
        }
    }
}

/// 解析完整持久记录；任何不完整或不自洽的输入都失败关闭。
///
/// # 错误
/// 超预算、半 JSON、未知字段返回 `CorruptStore`；未知版本返回 `UnknownSchema`。
pub fn decode(bytes: &[u8]) -> Result<TransportState> {
    if bytes.len() > STATE_MAX_BYTES {
        return Err(TransportError::Budget);
    }
    let first = bytes
        .iter()
        .copied()
        .find(|byte| !matches!(*byte, b' ' | b'\n' | b'\r' | b'\t'));
    if first != Some(b'{') {
        return Err(TransportError::CorruptStore);
    }
    // 先只取版本：未知版本必须报未知版本，而不是被未知字段吞成损坏。
    #[derive(Deserialize)]
    struct Version {
        schema_version: u8,
    }
    let version: Version =
        serde_json::from_slice(bytes).map_err(|_| TransportError::CorruptStore)?;
    if version.schema_version != SCHEMA_VERSION {
        return Err(TransportError::UnknownSchema);
    }
    let state: TransportState =
        serde_json::from_slice(bytes).map_err(|_| TransportError::CorruptStore)?;
    validate(&state)?;
    Ok(state)
}

/// 序列化完整记录；超过 64 KiB 拒绝发布。
///
/// # 错误
/// 记录不自洽返回 `CorruptStore`；编码超预算返回 `Budget`。
pub fn encode(state: &TransportState) -> Result<Vec<u8>> {
    validate(state)?;
    let bytes = serde_json::to_vec(state).map_err(|_| TransportError::CorruptStore)?;
    if bytes.len() > STATE_MAX_BYTES {
        return Err(TransportError::Budget);
    }
    Ok(bytes)
}

/// 完整记录的自洽检查；阶段与证据必须互相支撑。
///
/// # 错误
/// 任何不自洽返回 `CorruptStore`；未知版本返回 `UnknownSchema`。
pub fn validate(state: &TransportState) -> Result<()> {
    if state.schema_version != SCHEMA_VERSION {
        return Err(TransportError::UnknownSchema);
    }
    if state.generation == 0
        || state.next_sequence == 0
        || !state.budget_sane()
        || state.owner.uid == state.backend.payload_uid
        || state.owner.gid == state.backend.payload_gid
        || state.roots.private_identity == state.roots.mirror_identity
        // 视图路径与真实路径必须是两个名字；同名意味着 owner 直接拿到了 payload 身份的目录。
        || state.roots.mirror_view_root == state.roots.mirror_root
        || !super::approval::absolute_normal(&state.roots.mirror_view_root)
        || state.runtime.version.is_empty()
    {
        return Err(TransportError::CorruptStore);
    }
    if let Some(record) = &state.active {
        record_consistent(record, state.next_sequence)?;
        if state
            .last_terminal
            .as_ref()
            .is_some_and(|terminal| terminal.run_id == record.run_id)
        {
            return Err(TransportError::CorruptStore);
        }
    }
    if let Some(terminal) = &state.last_terminal
        && terminal.sequence >= state.next_sequence
    {
        return Err(TransportError::CorruptStore);
    }
    Ok(())
}

impl TransportState {
    fn budget_sane(&self) -> bool {
        self.active
            .as_ref()
            .is_none_or(|record| record.budget.complete())
    }
}
