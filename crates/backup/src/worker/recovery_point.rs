//! 恢复点年龄与显示状态。
//!
//! 年龄一律从内容截止时刻（`checkpoint_at`）算起，不从上传或提交完成时刻算。
//! 时钟回退或时间证据缺失显示未知；首次完整 seed 校验前显示「尚未建立灾难恢复点」。
//! 每日追加／不可变恢复点单独计龄；普通 SMB/NFS 目录不得显示具备不可变保护。

use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::{
    AgeUnknownReason, InstantMillis, RecoveryPointAge, recovery_point_age,
};

/// 一个恢复点的三个持久时间戳。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPointStamps {
    /// 内容截止时刻；年龄的起点。
    pub checkpoint_at: InstantMillis,
    /// 远端校验通过时刻。
    pub remote_verified_at: InstantMillis,
    /// 最终清单持久提交完成时刻。
    pub finalized_at: InstantMillis,
}

/// 恢复点类别；两类各自计龄，不能互相顶替。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPointClass {
    /// 按调度产生的普通恢复点。
    Scheduled,
    /// 每日追加／不可变恢复点。
    DailyImmutable,
}

/// 备份目标的形态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTargetKind {
    /// 本机临时仓库；只用于夹具，不构成灾难恢复目标。
    LocalTemporaryRepository,
    /// 普通 SMB/NFS 目录挂载。
    PlainSmbOrNfsDirectory,
    /// 专用的仅追加／不可变目标。
    DedicatedAppendOnlyTarget,
}

/// 目标不可变保护能力的判定结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ImmutabilityCapability {
    /// 已由真实目标证明。
    Proven,
    /// 未证明；界面不得显示具备不可变保护。
    NotProven {
        /// 未证明的原因。
        reason: ImmutabilityUnprovenReason,
    },
}

/// 不可变能力未被证明的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImmutabilityUnprovenReason {
    /// 普通 SMB/NFS 目录本身不提供不可变保护。
    PlainDirectoryTargetHasNoImmutability,
    /// 本机临时仓库只是夹具。
    LocalTemporaryFixture,
    /// 专用目标尚未接入核验。
    DedicatedTargetNotConnected,
}

impl BackupTargetKind {
    /// 依目标形态判定不可变能力；声明本身不能造出不可变保护。
    #[must_use]
    pub fn immutability(self, connected_and_proven: bool) -> ImmutabilityCapability {
        match self {
            Self::PlainSmbOrNfsDirectory => ImmutabilityCapability::NotProven {
                reason: ImmutabilityUnprovenReason::PlainDirectoryTargetHasNoImmutability,
            },
            Self::LocalTemporaryRepository => ImmutabilityCapability::NotProven {
                reason: ImmutabilityUnprovenReason::LocalTemporaryFixture,
            },
            Self::DedicatedAppendOnlyTarget => {
                if connected_and_proven {
                    ImmutabilityCapability::Proven
                } else {
                    ImmutabilityCapability::NotProven {
                        reason: ImmutabilityUnprovenReason::DedicatedTargetNotConnected,
                    }
                }
            }
        }
    }
}

/// 调度与完成预算相对 RPO 的校验。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RpoBudget {
    /// 调度间隔毫秒数。
    pub schedule_interval_millis: u64,
    /// 单次完成预算毫秒数。
    pub completion_budget_millis: u64,
    /// 目标 RPO 毫秒数。
    pub rpo_millis: u64,
}

impl RpoBudget {
    /// 默认 10 分钟调度 + 5 分钟完成预算，目标 RPO 15 分钟。
    pub const DEFAULT: Self = Self {
        schedule_interval_millis: 10 * 60 * 1000,
        completion_budget_millis: 5 * 60 * 1000,
        rpo_millis: 15 * 60 * 1000,
    };

    /// 间隔加完成预算不得超过 RPO；相加溢出一律判为不满足。
    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        self.schedule_interval_millis
            .checked_add(self.completion_budget_millis)
            .is_some_and(|total| total <= self.rpo_millis)
    }
}

/// 单个类别恢复点的显示状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RecoveryPointDisplay {
    /// 首次完整 seed 校验尚未通过：尚未建立灾难恢复点。
    NotEstablished,
    /// 已建立；附带按内容截止时刻计算的年龄。
    Established {
        /// 恢复点年龄。
        age: RecoveryPointAge,
    },
}

impl RecoveryPointDisplay {
    /// 是否可以在界面上显示为已具备灾难恢复能力。
    #[must_use]
    pub fn is_established(&self) -> bool {
        matches!(self, Self::Established { .. })
    }
}

/// 计算一类恢复点的显示状态。
#[must_use]
pub fn evaluate_class(
    seed_verified: bool,
    stamps: Option<RecoveryPointStamps>,
    now: Option<InstantMillis>,
) -> RecoveryPointDisplay {
    if !seed_verified {
        return RecoveryPointDisplay::NotEstablished;
    }
    let Some(stamps) = stamps else {
        return RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::MissingTimeEvidence,
            },
        };
    };
    RecoveryPointDisplay::Established {
        age: recovery_point_age(Some(stamps.checkpoint_at), now),
    }
}

/// 面向界面的恢复点整体状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPointStatus {
    /// 普通调度恢复点。
    pub scheduled: RecoveryPointDisplay,
    /// 每日追加／不可变恢复点，单独计龄。
    pub daily_immutable: RecoveryPointDisplay,
    /// 目标的不可变能力判定。
    pub immutability: ImmutabilityCapability,
    /// 当前预算是否满足 RPO。
    pub rpo_satisfied: bool,
}

/// 计算恢复点整体状态的输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryPointInputs {
    /// 首次完整 seed 是否已校验通过。
    pub seed_verified: bool,
    /// 最近一个普通调度恢复点。
    pub scheduled: Option<RecoveryPointStamps>,
    /// 最近一个每日追加／不可变恢复点。
    pub daily_immutable: Option<RecoveryPointStamps>,
    /// 备份目标形态。
    pub target: BackupTargetKind,
    /// 目标是否已接入并证明不可变能力。
    pub immutability_proven: bool,
    /// 当前时刻；缺失即年龄未知。
    pub now: Option<InstantMillis>,
    /// 调度与完成预算。
    pub budget: RpoBudget,
}

impl RecoveryPointStatus {
    /// 按输入计算整体状态。
    #[must_use]
    pub fn evaluate(inputs: RecoveryPointInputs) -> Self {
        Self {
            scheduled: evaluate_class(inputs.seed_verified, inputs.scheduled, inputs.now),
            daily_immutable: evaluate_class(
                inputs.seed_verified,
                inputs.daily_immutable,
                inputs.now,
            ),
            immutability: inputs.target.immutability(inputs.immutability_proven),
            rpo_satisfied: inputs.budget.is_satisfied(),
        }
    }

    /// 界面是否可以显示「具备不可变保护」。
    #[must_use]
    pub fn shows_immutable_protection(&self) -> bool {
        self.immutability == ImmutabilityCapability::Proven
    }
}
