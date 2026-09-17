//! 公开状态和可信观察 DTO；不持有文件系统或权限能力。
use super::outcome::EffectReceipt;
use crate::{Action, Baseline, Binding, Digest, Inventory, PlanningCursor};

/// 操作失败闭集；Unavailable 可能发生在提交之后，须沿原键重开核对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// 固定主库或租约缺失，不会自动补建。
    MissingStore,
    /// 初始化发现残留或现有实例。
    AlreadyExists,
    /// 私有权限、类型、容量或 inode 身份不符。
    UnsafeStorage,
    /// 另一个进程持有真实 owner 租约。
    Busy,
    /// 当前平台不具备所需文件边界。
    UnsupportedPlatform,
    /// 未知 schema 或二进制版本，不会自动迁移。
    UnsupportedSchema,
    /// 已存状态、类型或关系自相矛盾。
    CorruptStore,
    /// I/O 或数据库不可用；不能据此断言尚未提交。
    Unavailable,
    /// 实际或最坏终态容量超出冻结预算。
    Budget,
    /// 整数不能安全持久化或运算溢出。
    Overflow,
    /// Link、根或范围绑定不符。
    BindingMismatch,
    /// 预期版本、游标或外部数据库状态改变。
    VersionConflict,
    /// 旧 pending 尚未闭合。
    PendingUnresolved,
    /// 删除熔断保持生效，本接口不提供解除。
    Fused,
    /// 重试与冻结原输入不同。
    RetryMismatch,
    /// 权威审计未返回可信 ACK。
    AuditUnavailable,
    /// 动作尚无真实 Intent ACK。
    IntentRequired,
    /// 已派发动作尚无可靠终态。
    OutcomeUnknown,
    /// 回执与原键、条件或可信后端绑定不符。
    OutcomeMismatch,
    /// 旧 last 尚无持久清理票据及真实审计 ACK。
    RetentionRequired,
    /// 原键已正式退休；拒绝不代表曾成功。
    Retired,
    /// 当前保留状态没有该原键。
    NotFound,
    /// 结果、审计或共同观察尚不足以结束计划。
    NotSettled,
    /// 观察过期、跨重开实例或不完整。
    StaleObservation,
}

/// 控制状态快照；版本用于 CAS，不能充当授权凭据。
#[derive(Clone, Debug)]
pub struct StateView {
    /// 当前持久状态版本。
    pub version: u64,
    /// 原计划及两侧观察的单调游标。
    pub cursor: PlanningCursor,
    /// 最近一次完整共同基线。
    pub baseline: Baseline,
    /// 当前未闭合计划序号。
    pub pending_sequence: Option<u64>,
    /// 保留完整原键与回执的最近终态序号。
    pub last_sequence: Option<u64>,
    /// 已审计退休的序号下界。
    pub retired_through: u64,
    /// 是否存在不可自行解除的删除熔断。
    pub fused: bool,
}

/// 整批预留结果；Ready 仍须逐动作 Intent ACK 与执行前核验。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reservation {
    /// 已保存可继续核验的原计划。
    Ready {
        /// 已冻结的计划序号。
        sequence: u64,
    },
    /// 已原子保存熔断和游标。
    Fused {
        /// 触发熔断的计划序号。
        sequence: u64,
    },
}

/// 单动作原键状态；相同内容不能把未知改成已知。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionStatus {
    /// Intent 尚未由真实审计确认。
    IntentPending,
    /// Intent 已确认，尚未持久化 Issued。
    Reserved,
    /// Issued 已持久化，原键结果仍未知。
    OutcomeUnknown,
    /// 已保存经可信 adapter 核验的原回执。
    Known(Box<EffectReceipt>),
}

/// 单动作查询视图，不是新的执行许可。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressView {
    /// 冻结的纯计划动作。
    pub action: Action,
    /// 实际调用后端时使用的原幂等键。
    pub external_key: String,
    /// 当前持久动作状态。
    pub status: ActionStatus,
}

/// 仅 issue 持久化后返回；执行方不得替换 key、条件或源。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAction {
    /// 冻结的动作及 CAS 条件。
    pub action: Action,
    /// 重试必须沿用的后端幂等键。
    pub external_key: String,
    /// 原输入和动作位置绑定的请求摘要。
    pub request_digest: Digest,
}

/// 可信 adapter 的新鲜证据；本库不扫描磁盘，重开后须重新取得。
#[derive(Clone, Debug)]
pub struct ExecutionCheck {
    /// 本次实际核验的 Link、根与范围。
    pub binding: Binding,
    /// 完整观察中明确存在或缺失的源。
    pub source: crate::Presence,
    /// 完整观察中明确存在或缺失的目标。
    pub target: crate::Presence,
    /// Canonical 新观察序号。
    pub canonical_observation: u64,
    /// Mirror 新观察序号。
    pub mirror_observation: u64,
    /// 当前打开实例给出的内存标识。
    pub process_epoch: Digest,
    /// 单调时钟截止点；过期不取消已 Issued 请求。
    pub expires_at: std::time::Instant,
}

/// 完整双侧重查；transport 成功或相同内容不能替代动作回执。
#[derive(Clone, Debug)]
pub struct CommonObservation {
    /// Canonical 的完整新观察。
    pub canonical: Inventory,
    /// Mirror 的完整新观察。
    pub mirror: Inventory,
    /// 可信范围 adapter 对整个联合路径的证明输入。
    pub approved: crate::ApprovedScope,
    /// 当前打开实例给出的内存标识。
    pub process_epoch: Digest,
    /// 共同观察的单调截止点。
    pub expires_at: std::time::Instant,
}

/// 一次有界审计 flush 的实际 ACK 进度。
#[derive(Clone, Debug)]
pub struct AuditDelivery {
    /// 已核验并持久化 ACK 的事件数。
    pub acknowledged: usize,
    /// 是否仍有待审计事件。
    pub remaining: bool,
}
