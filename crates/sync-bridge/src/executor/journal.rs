//! 持久层与可信观察端口；执行器只按原键说话，绝不自带存储或时钟权限。
//!
//! `PlanJournal` 的实现由持久 owner 提供（接既有 BridgeState），
//! 逐动作状态与不可变 Input 分表、审计 append 在事务之外、退休永久拒绝等语义都在实现方，
//! 本 trait 只固定执行器需要的最小读写面，不新增任何绕过入口。

use super::quiescence::Quiesced;
use crate::{ActionKey, ApprovedScope, Binding, Digest, Inventory, Presence};
use std::time::Instant;

/// 持久层失败闭集；`Unavailable` 可能发生在提交之后，须沿原键重开核对。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalFault {
    /// 存储或数据库不可用；不能据此断言尚未提交。
    Unavailable,
    /// 预期版本或外部状态改变。
    VersionConflict,
    /// 当前保留状态没有该原键。
    NotFound,
    /// 原键已退休，永久拒绝。
    Retired,
    /// 删除熔断仍生效。
    Fused,
    /// 动作尚无真实 Intent ACK。
    IntentRequired,
    /// 回执与原键、条件或可信后端绑定不符。
    OutcomeMismatch,
    /// 观察过期、跨重开实例或不完整。
    StaleObservation,
    /// 实际或最坏终态容量超出冻结预算。
    Budget,
    /// Link、根或范围绑定不符。
    BindingMismatch,
    /// 结果、审计或共同观察尚不足以结束计划。
    NotSettled,
    /// 旧终态尚无持久清理票据及真实审计 ACK。
    RetentionRequired,
    /// 权威审计未返回可信 ACK。
    AuditUnavailable,
}

/// 可信观察 adapter 失败闭集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WitnessFault {
    /// 观察 adapter 当前不可用。
    Unavailable,
    /// 无法取得完整观察；缺失必须显式，不能用漏项表达。
    Incomplete,
    /// 观察已过期或跨了打开实例。
    Stale,
}

/// 单动作持久状态；相同内容不能把未知改成已知。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    /// Intent 尚未由真实审计确认。
    IntentPending,
    /// Intent 已确认，尚未持久化 Issued。
    Reserved,
    /// Issued 已持久化，原键结果仍未知。
    Issued,
    /// 已保存经核验的提交回执。
    Committed,
    /// 已保存经核验的明确无效果回执。
    NoEffect,
}

/// 逐动作快照；`external_key` 是实际调用后端的原幂等键。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    /// 冻结动作键。
    pub key: ActionKey,
    /// 持久化的后端幂等键；重试必须沿用。
    pub external_key: String,
    /// 当前持久状态。
    pub state: JobState,
}

/// 整批持久快照；version 用于 CAS，不是授权凭据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalSnapshot {
    /// 当前持久状态版本。
    pub version: u64,
    /// 当前未闭合计划序号。
    pub pending_sequence: Option<u64>,
    /// 当前共同基线版本。
    pub baseline_version: u64,
    /// 已审计退休的序号下界。
    pub retired_through: u64,
    /// 是否存在不可自行解除的删除熔断。
    pub fused: bool,
    /// 与冻结计划同序的逐动作状态。
    pub jobs: Vec<JobRecord>,
}

/// 执行前的新鲜双侧重验证据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionRecheck {
    /// 本次实际核验的 Link、根与范围。
    pub binding: Binding,
    /// 完整观察中明确存在或缺失的源。
    pub source: Presence,
    /// 完整观察中明确存在或缺失的目标。
    pub target: Presence,
    /// Canonical 新观察序号。
    pub canonical_observation: u64,
    /// Mirror 新观察序号。
    pub mirror_observation: u64,
    /// 当前打开实例给出的内存标识。
    pub witness_epoch: Digest,
    /// 单调时钟截止点；过期不取消已 Issued 请求。
    pub expires_at: Instant,
}

/// 整批完成后的完整共同重查；transport 成功不能替代它。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommonRecheck {
    /// Canonical 的完整新观察。
    pub canonical: Inventory,
    /// Mirror 的完整新观察。
    pub mirror: Inventory,
    /// 可信范围 adapter 对整个联合路径的证明输入。
    pub approved: ApprovedScope,
    /// 当前打开实例给出的内存标识。
    pub witness_epoch: Digest,
    /// 共同观察的单调截止点。
    pub expires_at: Instant,
}

/// 已持久化 Issued 的动作；执行方不得替换 key、条件或源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedJob {
    /// 冻结动作键。
    pub key: ActionKey,
    /// 后端幂等键。
    pub external_key: String,
    /// 原输入与动作位置绑定的请求摘要。
    pub request_digest: Digest,
}

/// 明确无安装效果的闭集原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoEffectCause {
    /// 原 CAS 条件未满足，未安装。
    CasRejected,
    /// 安装前取消已由后端持久确认。
    CancelledBeforeInstall,
}

/// 单动作终态；`Unknown` 永远不会被折叠成成功。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobOutcome {
    /// 后端确认原请求已持久提交。
    Committed {
        /// 删除为 None，其余为实际新 revision。
        new_revision: Option<Digest>,
        /// 非删除必须等于冻结源的内容摘要。
        content_hash: Option<Digest>,
        /// 非删除必须等于冻结源的字节数。
        bytes: Option<u64>,
        /// 后端核验过的真实持久回执标识。
        receipt_id: String,
    },
    /// 同原键明确无安装效果。
    NoEffect {
        /// 后端已确认的无效果原因。
        reason: NoEffectCause,
        /// 后端持久拒绝或取消票据标识。
        terminal_id: String,
    },
    /// 拿不到权威终态；重开后仍按原键查询，绝不补成成功。
    Unknown,
}

/// 一次有界审计 flush 的实际 ACK 进度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditProgress {
    /// 已核验并持久化 ACK 的事件数。
    pub acknowledged: usize,
    /// 是否仍有待审计事件。
    pub remaining: bool,
}

/// 持久 owner 端口；实现方负责事务、分表与审计顺序。
pub trait PlanJournal {
    /// 读取当前整批快照。
    fn snapshot(&self) -> Result<JournalSnapshot, JournalFault>;
    /// 真实 append 在事务之外，返回精确回执后才 CAS 保存 ACK。
    fn deliver_audit(&mut self, max_records: usize) -> Result<AuditProgress, JournalFault>;
    /// Intent ACK 与新鲜观察通过后持久化 Issued，再返回原键执行数据。
    fn issue(
        &mut self,
        key: &ActionKey,
        recheck: &ExecutionRecheck,
    ) -> Result<IssuedJob, JournalFault>;
    /// 保存经核验的原键终态；`Unknown` 不会走到这里。
    fn record_outcome(&mut self, job: &IssuedJob, outcome: &JobOutcome)
    -> Result<(), JournalFault>;
    /// 冻结上一份完整终态的清理票据；真实审计 ACK 之前不删除、不推进退休下界。
    fn prepare_retirement(&mut self, expected_version: u64) -> Result<(), JournalFault>;
    /// 闭合计划；`common` 为 None 时保留旧共同基线。
    fn settle(
        &mut self,
        expected_version: u64,
        sequence: u64,
        common: Option<CommonRecheck>,
    ) -> Result<(), JournalFault>;
}

/// 可信观察 adapter 端口；只有它能给出执行前与收尾的新鲜证据。
pub trait ExecutionWitness {
    /// 为一条动作取得执行前新鲜重验证据。
    fn recheck(
        &mut self,
        key: &ActionKey,
        quiesced: &Quiesced,
    ) -> Result<ExecutionRecheck, WitnessFault>;
    /// 取得整批收尾的完整共同观察；取不到时返回 None，不得补造。
    fn common(&mut self, quiesced: &Quiesced) -> Result<Option<CommonRecheck>, WitnessFault>;
}
