//! 接收生产者发的原键、读原键在本地状态里的位置、来源过期之后的出口。
//!
//! 序号由生产者发：意图在封存之前就要在人侧账本里，worker 只能照原键登记，不另造序号。
use super::{BackupWorker, JobOutcome, WorkerError, backup_intent, make_room};
use crate::store::model::StoreError;
use crate::worker::state::{FailureReason, JobKey, JobRecord, JobState, UnknownStage};
use studiolo_audit_ledger::{AuditFilter, AuditPhase};

/// 原键在本地状态里的位置；常驻循环据此决定下一步，不自己读状态文件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPosition {
    /// 本地还没有这条作业。
    Absent,
    /// 已接收，还没开始。
    Queued,
    /// 上传或数据复验没做完（含崩溃后标成未知的）；只能只读核对，或按来源过期处理。
    Interrupted,
    /// 最终清单草稿已经持久；只做非正文核对和补记。
    Finalizing,
    /// 已经失败，失败结果还没写进账本。
    FailurePending,
    /// 终态，结果已经在账本里。
    Finished,
}

impl BackupWorker<'_> {
    /// 原键现在走到哪一步；只读。
    pub fn position(&self, key: &JobKey) -> Result<JobPosition, WorkerError> {
        let state = self.state()?;
        let Some(job) = state.jobs.iter().find(|job| &job.key == key) else {
            return Ok(JobPosition::Absent);
        };
        if state
            .pending_failure
            .as_ref()
            .is_some_and(|pending| &pending.key == key)
        {
            return Ok(JobPosition::FailurePending);
        }
        Ok(match job.state {
            JobState::Queued => JobPosition::Queued,
            JobState::Sealing
            | JobState::Uploading
            | JobState::Verifying
            | JobState::ResultUnknown {
                stage: UnknownStage::Sealing | UnknownStage::Uploading | UnknownStage::Verifying,
            } => JobPosition::Interrupted,
            JobState::CommittingFinalManifest
            | JobState::AuditPending { .. }
            | JobState::ResultUnknown {
                stage: UnknownStage::FinalManifestAck,
            } => JobPosition::Finalizing,
            JobState::Succeeded | JobState::Failed { .. } => JobPosition::Finished,
        })
    }

    /// 接收生产者发的原键：原意图必须已经在账本里，只读核对，不追加。
    ///
    /// 已经登记过就是幂等的。序号比本地见过的旧、或同一个 checkpoint 换了序号都拒绝：
    /// 那说明账本和本地状态不是同一条历史（例如账本是从旧备份恢复的），要人来处理。
    pub fn accept(&self, key: &JobKey) -> Result<(), WorkerError> {
        let mut state = self.state()?;
        if state.jobs.iter().any(|job| &job.key == key) {
            return Ok(());
        }
        if key.sequence.get() < state.next_sequence
            || state
                .jobs
                .iter()
                .any(|job| job.key.checkpoint_id == key.checkpoint_id)
        {
            return Err(WorkerError::EvidenceMismatch);
        }
        let expected = backup_intent(key, self.target)?;
        let found = self
            .ledger
            .query(
                &AuditFilter::default()
                    .with_event_id(expected.event_id().clone())
                    .with_phase(AuditPhase::Intent)
                    .with_limit(1),
            )
            .map_err(|error| WorkerError::Audit(error.into()))?;
        let [record] = found.as_slice() else {
            return Err(WorkerError::EvidenceMismatch);
        };
        record
            .verify_integrity()
            .map_err(|error| WorkerError::Audit(error.into()))?;
        if record.event() != &expected {
            return Err(WorkerError::EvidenceMismatch);
        }
        make_room(&mut state)?;
        state.next_sequence = key
            .sequence
            .get()
            .checked_add(1)
            .ok_or(StoreError::LimitExceeded)?;
        state.jobs.push(JobRecord::queued(key.clone()));
        self.store.save_state(&state)?;
        Ok(())
    }

    /// 来源过了销毁期限之后的出口：只对还没有最终清单草稿的作业，记成失败并补失败结果。
    ///
    /// 调用方必须已经用原准入事实确认过期。这里不重传，也不去猜远端有没有留下数据快照；
    /// 没有最终清单的数据快照本来就不算恢复点。
    pub fn expire(&self, key: &JobKey) -> Result<JobOutcome, WorkerError> {
        match self.position(key)? {
            JobPosition::Queued | JobPosition::Interrupted => {}
            JobPosition::Absent => return Err(WorkerError::JobNotFound),
            _ => return Err(WorkerError::EvidenceMismatch),
        }
        let state = self.state()?;
        if state.pending_failure.is_some()
            || state.pending_final.is_some()
            || state.jobs.iter().any(|job| {
                &job.key != key && job.state != JobState::Queued && !job.state.is_terminal()
            })
        {
            return Err(WorkerError::JobBusy);
        }
        self.finish_failed(key, FailureReason::SourceExpired, None)?;
        Ok(JobOutcome::Failed(FailureReason::SourceExpired))
    }
}
