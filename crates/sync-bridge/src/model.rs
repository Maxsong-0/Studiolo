//! 条件动作与共同观察候选；没有成功回执或授权能力。

pub use super::evidence::*;
use studiolo_core::VaultPath;

/// 不能用内容 hash 替代的单调提案键。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanKey {
    /// 所属同步链。
    pub link_id: Identity,
    /// 规则代次。
    pub scope_epoch: u64,
    /// 持久 owner 分配的计划序号。
    pub sequence: u64,
}

/// 一条条件动作的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// 只能创建不存在的目标。
    CreateOnly,
    /// 只能替换指定当前版本。
    Replace,
    /// 只能保全后删除指定当前版本。
    Delete,
}

/// 同一冻结动作重试时必须完整保持的键。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionKey {
    /// 原提案键。
    pub plan: PlanKey,
    /// 原逻辑路径，不使用额外整数映射。
    pub path: VaultPath,
    /// 条件写入所在侧。
    pub target: Side,
    /// 动作种类。
    pub operation: OperationKind,
}

/// 执行者仍须重验两侧证据；本核不执行或确认动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// 目标必须仍缺失。
    CreateOnly {
        /// 另一侧仍须为这个源版本。
        source: FileVersion,
    },
    /// 目标必须仍为指定版本。
    Replace {
        /// 另一侧仍须为这个源版本。
        source: FileVersion,
        /// 条件替换的目标版本。
        target_expected: FileVersion,
    },
    /// 执行前保全原版本及至少七天的可恢复删除记录。
    Delete {
        /// 条件删除的目标版本；源侧仍须缺失。
        target_expected: FileVersion,
    },
}

/// 不具备执行权限的一条提案。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// 稳定动作键及目标侧。
    pub key: ActionKey,
    /// 源总是另一侧，执行者须重验原观察。
    pub change: Change,
}

/// 需要保全双方而不能自动选赢家的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictReason {
    /// 首次没有共同基线，双方内容不同。
    FirstDivergence,
    /// 相对共同基线双方分别修改。
    BothChanged,
    /// 一侧删除，另一侧修改。
    DeleteAgainstModification,
}

/// 只保留元数据；不能冒称冲突字节已经落盘。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// 冲突路径。
    pub path: VaultPath,
    /// 冲突原因。
    pub reason: ConflictReason,
    /// 原共同基线或首次无基线。
    pub baseline: Option<Pair>,
    /// 双方当前证据，包括明确缺失。
    pub observed: Pair,
}

/// 共同观察建议；不能确认旧未决动作或直接推进基线。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgreementCandidate {
    /// 观察路径。
    pub path: VaultPath,
    /// 双方当前证据。
    pub observed: Pair,
    /// 持久层另行核对的旧基线版本。
    pub expected_baseline_version: u64,
}

/// 全批统计，不以新文件扩充分母。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteStats {
    /// 拟删除 canonical 对象数。
    pub canonical: u64,
    /// 拟删除镜像对象数。
    pub mirror: u64,
    /// 原完整共同基线的存活路径数。
    pub baseline_live_paths: u64,
    /// 所有复制候选的实际字节数，供传输层另行预算。
    pub copy_bytes: u64,
}

/// 已触发的全批门槛。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuseReason {
    /// 超过数量门槛。
    Count,
    /// 超过比例门槛。
    Ratio,
    /// 两个门槛均超过。
    CountAndRatio,
}

/// 整批阻断结果，没有可执行动作字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuseBlocked {
    /// 本次提案键。
    pub key: PlanKey,
    /// 全批统计。
    pub stats: DeleteStats,
    /// 触发原因。
    pub reason: FuseReason,
    /// 仍需保全的冲突元数据。
    pub conflicts: Vec<Conflict>,
}

/// 完整验证且未熔断的冻结提案，仍不代表真实同步。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub(super) original: Input,
    pub(super) key: PlanKey,
    pub(super) actions: Vec<Action>,
    pub(super) agreements: Vec<AgreementCandidate>,
    pub(super) conflicts: Vec<Conflict>,
    pub(super) stats: DeleteStats,
}

impl Plan {
    /// 原计划键。
    pub fn key(&self) -> &PlanKey {
        &self.key
    }
    /// 同键重试不得替换的完整证据。
    pub fn frozen_input(&self) -> &Input {
        &self.original
    }
    /// 条件动作候选。
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    /// 不包含回执的共同观察建议。
    pub fn agreements(&self) -> &[AgreementCandidate] {
        &self.agreements
    }
    /// 不会自动修改的冲突证据。
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }
    /// 全批预算统计。
    pub fn stats(&self) -> DeleteStats {
        self.stats
    }
}

/// 纯规划成功完成后的闭合结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// 条件动作待持久化和执行前核验。
    Ready(Box<Plan>),
    /// 整批没有可执行动作；本核没有解除入口。
    FuseBlocked(FuseBlocked),
}

/// 不产生部分动作的封闭失败原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// 任一全批资源预算不满足。
    Budget,
    /// 链路、根、Vault 或 scope 不一致。
    BindingMismatch,
    /// 观察侧标签错误。
    WrongSide,
    /// 任一侧没有完整观察。
    IncompleteObservation,
    /// 路径省略或不在精确范围内。
    ScopeMismatch,
    /// 基线、显式预期和游标不一致。
    BaselineVersionMismatch,
    /// 基线不是真正共同状态。
    InvalidBaseline,
    /// 计划或观察序号不符合前进条件。
    SequenceMismatch,
    /// revision/hash/字节数相互矛盾。
    EvidenceContradiction,
    /// 精确整数统计不能表示。
    Overflow,
    /// 旧计划尚未核对，不允许启动新规划。
    PendingUnresolved,
    /// 重试输入不等于原冻结输入。
    RetryMismatch,
}
