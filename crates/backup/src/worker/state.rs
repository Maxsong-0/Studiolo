//! BackupWorker 的作业键与状态机。
//!
//! 状态区分排队／封存／上传／验证／最终清单提交／成功／失败／结果未知／审计待确认。
//! 上传退出 0 只是中间证据；只有远端对象完整、校验通过、成对 checkpoint 引用的
//! 最终 manifest 持久提交之后，才存在可恢复点。
//!
//! 作业键与证据（`evidence` 子模块）、驱动事件与失败模式（`event` 子模块）按职责拆出，
//! 仍经本模块原路径导出。

mod event;
mod evidence;

pub use event::{EventKind, JobEvent, StateError};
pub use evidence::{
    FailureReason, JobKey, ReconciledOutcome, RemoteVerification, UnknownStage, UploadEvidence,
};

use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::InstantMillis;

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
