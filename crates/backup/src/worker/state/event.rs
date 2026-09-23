//! 驱动状态机的事件、错误消息用的事件类别，以及状态机的失败模式。

use super::{
    FailureReason, JobState, ReconciledOutcome, RemoteVerification, UnknownStage, UploadEvidence,
};
use studiolo_core::checkpoint::InstantMillis;

/// 驱动状态机的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobEvent {
    /// 开始封存，并记录内容截止时刻。
    StartSealing {
        /// 内容截止时刻。
        checkpoint_at: InstantMillis,
    },
    /// 封存完成，进入上传。
    SealCompleted,
    /// 上传进程退出；无论退出码如何都只能进入校验。
    UploadExited {
        /// 上传证据。
        evidence: UploadEvidence,
    },
    /// 远端校验完成。
    RemoteVerified {
        /// 校验结果。
        verification: RemoteVerification,
    },
    /// 最终清单已持久提交。
    FinalManifestCommitted {
        /// 提交完成时刻。
        finalized_at: InstantMillis,
        /// Outcome 审计是否成功写入。
        outcome_audit_succeeded: bool,
    },
    /// `AuditPending` 的固定补记已成功写入。
    OutcomeBackfilled,
    /// 明确失败。
    Failed {
        /// 失败原因。
        reason: FailureReason,
    },
    /// 结果未知。
    ResultUnknown {
        /// 未知阶段。
        stage: UnknownStage,
    },
    /// 沿原作业键核对未知结果。
    ReconcileUnknown {
        /// 核对结论。
        outcome: ReconciledOutcome,
    },
}

/// 事件类别，用于错误消息，不携带载荷。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// 开始封存。
    StartSealing,
    /// 封存完成。
    SealCompleted,
    /// 上传退出。
    UploadExited,
    /// 远端校验完成。
    RemoteVerified,
    /// 最终清单已提交。
    FinalManifestCommitted,
    /// 补记完成。
    OutcomeBackfilled,
    /// 明确失败。
    Failed,
    /// 结果未知。
    ResultUnknown,
    /// 核对未知结果。
    ReconcileUnknown,
}

impl EventKind {
    pub(super) fn of(event: JobEvent) -> Self {
        match event {
            JobEvent::StartSealing { .. } => Self::StartSealing,
            JobEvent::SealCompleted => Self::SealCompleted,
            JobEvent::UploadExited { .. } => Self::UploadExited,
            JobEvent::RemoteVerified { .. } => Self::RemoteVerified,
            JobEvent::FinalManifestCommitted { .. } => Self::FinalManifestCommitted,
            JobEvent::OutcomeBackfilled => Self::OutcomeBackfilled,
            JobEvent::Failed { .. } => Self::Failed,
            JobEvent::ResultUnknown { .. } => Self::ResultUnknown,
            JobEvent::ReconcileUnknown { .. } => Self::ReconcileUnknown,
        }
    }
}

/// 状态机失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    /// 非法状态迁移。
    IllegalTransition {
        /// 起始状态。
        from: JobState,
        /// 触发事件。
        event: EventKind,
    },
    /// 核对未知结果时作业键不符。
    JobKeyMismatch,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalTransition { from, event } => {
                write!(formatter, "状态 {from:?} 不接受事件 {event:?}")
            }
            Self::JobKeyMismatch => formatter.write_str("未知结果必须沿原作业键核对"),
        }
    }
}

impl std::error::Error for StateError {}
