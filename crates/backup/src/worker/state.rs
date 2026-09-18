//! BackupWorker 的作业键与状态机。
//!
//! 状态区分排队／封存／上传／验证／最终清单提交／成功／失败／结果未知／审计待确认。
//! 上传退出 0 只是中间证据；只有远端对象完整、校验通过、成对 checkpoint 引用的
//! 最终 manifest 持久提交之后，才存在可恢复点。

use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

/// 作业键：单调序号加 checkpoint 标识。不用内容 hash 当作业键。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobKey {
    /// 单调递增的作业序号。
    pub sequence: NonZeroU64,
    /// 本作业封存的 checkpoint。
    pub checkpoint_id: CheckpointId,
}

/// 上传进程留下的证据。退出 0 本身不构成可恢复点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadEvidence {
    /// 上传进程是否以 0 退出。
    pub exit_status_zero: bool,
    /// 上传进程自报的对象数；只是中间证据。
    pub reported_objects: u64,
}

/// 远端校验结果；三项全真才可能产生可恢复点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteVerification {
    /// 远端对象是否完整。
    pub objects_complete: bool,
    /// 内容校验是否通过。
    pub checksum_verified: bool,
    /// 最终 manifest 是否成对引用了本次 checkpoint。
    pub paired_checkpoint_reference: bool,
    /// 远端校验通过的时刻。
    pub verified_at: InstantMillis,
}

impl RemoteVerification {
    /// 是否构成可恢复点的必要条件。
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        self.objects_complete && self.checksum_verified && self.paired_checkpoint_reference
    }
}

/// 结果未知发生在哪个阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownStage {
    /// 封存阶段结果未知。
    Sealing,
    /// 上传阶段结果未知（例如上传中断）。
    Uploading,
    /// 远端校验阶段结果未知。
    Verifying,
    /// 最终清单提交后 ACK 丢失。
    FinalManifestAck,
}

/// 失败原因；只记录可公开的固定类别，不带正文或秘密。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// 跨 owner 屏障没有成立。
    BarrierNotEstablished,
    /// 封存前来源发生变化。
    SourceChangedBeforeSeal,
    /// 磁盘空间不足。
    OutOfSpace,
    /// 上传被拒绝或明确失败。
    UploadRejected,
    /// 远端校验不通过。
    RemoteVerificationFailed,
    /// 最终清单提交失败。
    FinalManifestCommitFailed,
    /// 备份目标未接入，无法验证。
    TargetUnverified,
}

/// 未知结果沿原作业键核对后的结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReconciledOutcome {
    /// 远端确认没有该作业键的对象。
    RemoteAbsent,
    /// 远端确认存在且通过校验。
    RemoteVerified {
        /// 远端校验结果。
        verification: RemoteVerification,
    },
}

/// 作业状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum JobState {
    /// 排队。
    Queued,
    /// 正在跨 owner 封存。
    Sealing,
    /// 正在上传。
    Uploading,
    /// 正在校验远端。
    Verifying,
    /// 正在持久提交最终清单。
    CommittingFinalManifest,
    /// 成功；此时才存在可恢复点。
    Succeeded,
    /// 明确失败。
    Failed {
        /// 失败原因。
        reason: FailureReason,
    },
    /// 结果未知；必须沿原作业键核对，不能当成失败或成功。
    ResultUnknown {
        /// 未知发生的阶段。
        stage: UnknownStage,
    },
    /// 远端已确认，但 Outcome 审计失败，等待固定补记。
    AuditPending {
        /// 远端校验结果，保留真实状态。
        verification: RemoteVerification,
    },
}

impl JobState {
    /// 是否为终态。`AuditPending` 不是终态，也不推进成功指针。
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed { .. })
    }

    /// 界面上的成功指针只认 `Succeeded`。
    #[must_use]
    pub fn advances_success_pointer(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

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

/// 一条作业记录；持久状态与三个时间戳都在这里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRecord {
    /// 作业键。
    pub key: JobKey,
    /// 当前状态。
    pub state: JobState,
    /// 内容截止时刻。
    pub checkpoint_at: Option<InstantMillis>,
    /// 远端校验通过时刻。
    pub remote_verified_at: Option<InstantMillis>,
    /// 最终清单持久提交完成时刻。
    pub finalized_at: Option<InstantMillis>,
}

impl JobRecord {
    /// 新建一条排队中的作业。
    #[must_use]
    pub fn queued(key: JobKey) -> Self {
        Self {
            key,
            state: JobState::Queued,
            checkpoint_at: None,
            remote_verified_at: None,
            finalized_at: None,
        }
    }

    /// 推进状态机；非法迁移一律拒绝，不会把中间证据当成结论。
    pub fn advance(&mut self, event: JobEvent) -> Result<(), StateError> {
        let next = self.next_state(event)?;
        self.apply_timestamps(event);
        self.state = next;
        Ok(())
    }

    fn next_state(&self, event: JobEvent) -> Result<JobState, StateError> {
        let illegal = || StateError::IllegalTransition {
            from: self.state,
            event: EventKind::of(event),
        };
        Ok(match (self.state, event) {
            (_, JobEvent::Failed { reason }) if !self.state.is_terminal() => {
                JobState::Failed { reason }
            }
            (_, JobEvent::ResultUnknown { stage }) if !self.state.is_terminal() => {
                JobState::ResultUnknown { stage }
            }
            (JobState::Queued, JobEvent::StartSealing { .. }) => JobState::Sealing,
            (JobState::Sealing, JobEvent::SealCompleted) => JobState::Uploading,
            // 退出 0 只是中间证据：无论如何都只能进入校验，不能直接成功。
            (JobState::Uploading, JobEvent::UploadExited { evidence }) => {
                if evidence.exit_status_zero {
                    JobState::Verifying
                } else {
                    JobState::Failed {
                        reason: FailureReason::UploadRejected,
                    }
                }
            }
            (JobState::Verifying, JobEvent::RemoteVerified { verification }) => {
                if verification.is_recoverable() {
                    JobState::CommittingFinalManifest
                } else {
                    JobState::Failed {
                        reason: FailureReason::RemoteVerificationFailed,
                    }
                }
            }
            (
                JobState::CommittingFinalManifest,
                JobEvent::FinalManifestCommitted {
                    outcome_audit_succeeded,
                    ..
                },
            ) => {
                if outcome_audit_succeeded {
                    JobState::Succeeded
                } else {
                    let Some(verified_at) = self.remote_verified_at else {
                        return Err(illegal());
                    };
                    JobState::AuditPending {
                        verification: RemoteVerification {
                            objects_complete: true,
                            checksum_verified: true,
                            paired_checkpoint_reference: true,
                            verified_at,
                        },
                    }
                }
            }
            (JobState::AuditPending { .. }, JobEvent::OutcomeBackfilled) => JobState::Succeeded,
            (
                JobState::ResultUnknown { .. },
                JobEvent::ReconcileUnknown {
                    outcome: ReconciledOutcome::RemoteAbsent,
                },
            ) => JobState::Failed {
                reason: FailureReason::UploadRejected,
            },
            (
                JobState::ResultUnknown { .. },
                JobEvent::ReconcileUnknown {
                    outcome: ReconciledOutcome::RemoteVerified { verification },
                },
            ) => {
                if verification.is_recoverable() {
                    JobState::CommittingFinalManifest
                } else {
                    JobState::Failed {
                        reason: FailureReason::RemoteVerificationFailed,
                    }
                }
            }
            _ => return Err(illegal()),
        })
    }

    fn apply_timestamps(&mut self, event: JobEvent) {
        match event {
            JobEvent::StartSealing { checkpoint_at } => self.checkpoint_at = Some(checkpoint_at),
            JobEvent::RemoteVerified { verification }
            | JobEvent::ReconcileUnknown {
                outcome: ReconciledOutcome::RemoteVerified { verification },
            } => {
                if verification.is_recoverable() {
                    self.remote_verified_at = Some(verification.verified_at);
                }
            }
            JobEvent::FinalManifestCommitted { finalized_at, .. } => {
                self.finalized_at = Some(finalized_at);
            }
            _ => {}
        }
    }

    /// 沿原作业键核对未知结果；作业键不同即拒绝。
    pub fn reconcile_unknown(
        &mut self,
        key: &JobKey,
        outcome: ReconciledOutcome,
    ) -> Result<(), StateError> {
        if *key != self.key {
            return Err(StateError::JobKeyMismatch);
        }
        if !matches!(self.state, JobState::ResultUnknown { .. }) {
            return Err(StateError::IllegalTransition {
                from: self.state,
                event: EventKind::ReconcileUnknown,
            });
        }
        self.advance(JobEvent::ReconcileUnknown { outcome })
    }
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
    fn of(event: JobEvent) -> Self {
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
