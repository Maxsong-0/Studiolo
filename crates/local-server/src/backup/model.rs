//! 界面文案与严重度。未知永远不是绿色，未验证永远不写成「已通过」。

use super::evidence::BackupSnapshot;
use serde::Serialize;
use studiolo_backup::worker::recovery_point::{
    ImmutabilityCapability, ImmutabilityUnprovenReason, RecoveryPointDisplay, RecoveryPointStatus,
};
use studiolo_backup::worker::state::JobState;
use studiolo_core::checkpoint::{AgeUnknownReason, RecoveryPointAge};

/// 一行状态的严重度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 已验证且当前良好。
    Ok,
    /// 证据不足，无法判断；界面不得显示为绿色。
    Unknown,
    /// 明确需要处理。
    Attention,
}

/// 健康页里的备份小节。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackupSection {
    /// 整体严重度。
    pub severity: Severity,
    /// 普通调度恢复点的一句话。
    pub recovery_point: String,
    /// 每日追加／不可变恢复点的一句话。
    pub daily_immutable: String,
    /// 不可变保护的一句话。
    pub immutable_protection: String,
    /// 调度预算相对 RPO 的一句话。
    pub rpo: String,
    /// 最近一次作业的一句话。
    pub last_job: String,
}

impl BackupSection {
    /// 没有任何证据时的显示；一律未知，不显示绿色。
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            severity: Severity::Unknown,
            recovery_point: "尚未建立灾难恢复点".to_owned(),
            daily_immutable: "尚未建立灾难恢复点".to_owned(),
            immutable_protection: "未证明具备不可变保护".to_owned(),
            rpo: "恢复点预算未知".to_owned(),
            last_job: "没有备份作业记录".to_owned(),
        }
    }

    /// 由快照生成显示。
    #[must_use]
    pub fn from_snapshot(snapshot: &BackupSnapshot) -> Self {
        if !snapshot.approved {
            let mut section = Self::unavailable();
            section.last_job = "备份未获主人批准，未启用".to_owned();
            return section;
        }
        let status = snapshot.status;
        Self {
            severity: severity(&status, snapshot.last_job),
            recovery_point: describe(status.scheduled),
            daily_immutable: describe(status.daily_immutable),
            immutable_protection: describe_immutability(status.immutability),
            rpo: if status.rpo_satisfied {
                "调度与完成预算满足声明的 RPO".to_owned()
            } else {
                "调度与完成预算不满足声明的 RPO".to_owned()
            },
            last_job: describe_job(snapshot.last_job),
        }
    }
}

fn describe(display: RecoveryPointDisplay) -> String {
    match display {
        RecoveryPointDisplay::NotEstablished => "尚未建立灾难恢复点".to_owned(),
        RecoveryPointDisplay::Established { age } => match age {
            RecoveryPointAge::Known { millis } => {
                format!("恢复点年龄 {} 秒（自内容截止时刻起算）", millis / 1_000)
            }
            RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::ClockWentBackwards,
            } => "恢复点年龄未知：本地时钟发生回退".to_owned(),
            RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::MissingTimeEvidence,
            } => "恢复点年龄未知：缺少时间证据".to_owned(),
        },
    }
}

fn describe_immutability(capability: ImmutabilityCapability) -> String {
    match capability {
        ImmutabilityCapability::Proven => "已对真实目标核验不可变保护".to_owned(),
        ImmutabilityCapability::NotProven { reason } => match reason {
            ImmutabilityUnprovenReason::PlainDirectoryTargetHasNoImmutability => {
                "未证明具备不可变保护：普通 SMB/NFS 目录不提供该能力".to_owned()
            }
            ImmutabilityUnprovenReason::LocalTemporaryFixture => {
                "未证明具备不可变保护：当前只是本机临时仓库夹具".to_owned()
            }
            ImmutabilityUnprovenReason::DedicatedTargetNotConnected => {
                "未证明具备不可变保护：专用目标尚未接入核验".to_owned()
            }
        },
    }
}

fn describe_job(state: Option<JobState>) -> String {
    match state {
        None => "没有备份作业记录".to_owned(),
        Some(JobState::Queued) => "最近一次作业排队中".to_owned(),
        Some(JobState::Sealing) => "最近一次作业正在跨 owner 封存".to_owned(),
        Some(JobState::Uploading) => "最近一次作业正在上传".to_owned(),
        Some(JobState::Verifying) => "最近一次作业正在校验远端".to_owned(),
        Some(JobState::CommittingFinalManifest) => "最近一次作业正在提交最终清单".to_owned(),
        Some(JobState::Succeeded) => "最近一次作业成功".to_owned(),
        Some(JobState::Failed { reason }) => format!("最近一次作业失败：{reason:?}"),
        Some(JobState::ResultUnknown { stage }) => {
            format!("最近一次作业结果未知（{stage:?}），需沿原作业键核对")
        }
        Some(JobState::AuditPending { .. }) => {
            "远端已确认但 Outcome 审计失败，等待固定补记；成功指针未前移".to_owned()
        }
    }
}

fn severity(status: &RecoveryPointStatus, last_job: Option<JobState>) -> Severity {
    if matches!(last_job, Some(JobState::Failed { .. })) {
        return Severity::Attention;
    }
    if !status.rpo_satisfied {
        return Severity::Attention;
    }
    if matches!(
        last_job,
        Some(JobState::ResultUnknown { .. } | JobState::AuditPending { .. })
    ) {
        return Severity::Unknown;
    }
    match status.scheduled {
        RecoveryPointDisplay::NotEstablished => Severity::Unknown,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Unknown { .. },
        } => Severity::Unknown,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Known { .. },
        } => Severity::Ok,
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
