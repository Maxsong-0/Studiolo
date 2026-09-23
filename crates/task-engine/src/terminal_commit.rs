//! 终态报告：T1 持久预留、「manifest 最后写入」的 T2 前滚与审计结果补投，以及预留计划的解码。
use super::*;

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    /// `TaskEngine::report` 的实现体；接缝实现只转发到这里。
    pub(super) fn report_inner(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError> {
        self.verify_vault_binding()?;
        let event_id = AuditEventId::parse(event_id.to_owned()).map_err(map_audit)?;
        self.migrate_terminal_bodies()?;
        let prior = self
            .control_db
            .attempt(report.task_id, report.attempt_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("attempt"))?;
        if prior.fence != report.fence {
            return Err(TaskEngineError::StaleFence);
        }
        if self
            .control_db
            .board_task(report.task_id)
            .map_err(map_db)?
            .is_some()
        {
            return Err(TaskEngineError::InvalidTaskSpec(
                "board tasks require structured board receipt",
            ));
        }
        if prior.phase == AttemptPhase::Running
            && !self
                .control_db
                .is_attempt_active_at(report.task_id, report.attempt_id, report.fence, now_ms)
                .map_err(map_db)?
        {
            return Err(TaskEngineError::StaleFence);
        }
        if !matches!(
            prior.phase,
            AttemptPhase::Running | AttemptPhase::Committing | AttemptPhase::Completed
        ) {
            return Err(TaskEngineError::AlreadyTerminal);
        }
        let reservation = self.stage_reservation(&event_id, &report)?;
        let replayed = match prior.phase {
            AttemptPhase::Running => {
                self.control_db
                    .reserve_terminal(
                        report.task_id,
                        report.attempt_id,
                        report.fence,
                        &reservation,
                        now_ms,
                    )
                    .map_err(map_db)?;
                false
            }
            AttemptPhase::Committing | AttemptPhase::Completed => {
                if prior.terminal.as_ref() != Some(&reservation) {
                    return Err(TaskEngineError::IdempotencyMismatch);
                }
                prior.phase == AttemptPhase::Completed
            }
            AttemptPhase::Claimed
            | AttemptPhase::CancelRequested
            | AttemptPhase::Failed
            | AttemptPhase::Cancelled
            | AttemptPhase::Abandoned => return Err(TaskEngineError::AlreadyTerminal),
        };
        let receipt = if replayed {
            let receipt = self
                .control_db
                .artifact_receipt(prior.task_id, prior.attempt_id)
                .map_err(map_db)?
                .ok_or_else(|| {
                    TaskEngineError::Integrity("completed attempt has no receipt".to_owned())
                })?;
            self.deliver_pending_outcomes(Some(prior.attempt_id), now_ms)?;
            serde_json::from_value(receipt)?
        } else {
            self.roll_forward(&prior_identity(&prior), &reservation, now_ms)?
        };
        Ok(ReportResult {
            status: TaskStatus::NeedsReview,
            receipt,
            replayed,
        })
    }

    pub(super) fn roll_forward(
        &mut self,
        attempt: &AttemptIdentity,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        let plan = self.staged_plan(attempt, reservation)?;
        let stopped = if reservation
            .report_json
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            == Some("stopped")
        {
            Some(self.stopped_receipt_from_plan(attempt, &plan)?)
        } else {
            None
        };
        let subject = AuditSubject::default()
            .with_task(attempt.task_id)
            .with_attempt(attempt.attempt_id);
        let intent_payload = SanitizedPayload::try_from(json!({
            "fence": attempt.fence.get(),
            "artifact_count": plan.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id.clone(),
                    AuditPhase::Intent,
                    "task_engine.report",
                    intent_payload,
                )
                .with_subject(subject),
            )
            .map_err(map_audit)?;

        let commit = ArtifactCommit {
            task_id: attempt.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            audit_intent: plan.event_id.to_string(),
            idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
            artifacts: plan.artifacts,
        };
        let receipt = self
            .broker
            .commit_attempt_artifacts(commit)
            .map_err(TaskEngineError::CommitFailed)?;

        let encoded_receipt = serde_json::to_value(&receipt)?;
        if let Some(stopped) = stopped {
            self.finish_stopped_candidates(attempt, stopped, &receipt, now_ms)?;
        } else {
            self.control_db
                .finalize_terminal(
                    attempt.task_id,
                    attempt.attempt_id,
                    attempt.fence,
                    TaskStatus::NeedsReview,
                    Some(&encoded_receipt),
                    now_ms,
                )
                .map_err(map_db)?;
        }
        // 只有在权威 T2 提交之后，审计才能宣称工作流状态。
        // T2 的 outbox 能扛过恰好发生在此边界的崩溃或账本不可用。
        self.deliver_pending_outcomes(Some(attempt.attempt_id), now_ms)?;
        Ok(receipt)
    }

    pub(super) fn deliver_pending_outcomes(
        &mut self,
        only_attempt: Option<AttemptId>,
        now_ms: i64,
    ) -> Result<usize, TaskEngineError> {
        let pending = self.control_db.pending_terminal_audits().map_err(map_db)?;
        let mut delivered = 0;
        for outcome in pending {
            if only_attempt.is_some_and(|id| id != outcome.attempt.attempt_id) {
                continue;
            }
            self.deliver_outcome(&outcome)?;
            self.control_db
                .mark_terminal_audit_delivered(outcome.attempt.attempt_id, now_ms)
                .map_err(map_db)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn deliver_outcome(&self, outcome: &PendingTerminalAudit) -> Result<(), TaskEngineError> {
        let reservation = outcome.attempt.terminal.as_ref().ok_or_else(|| {
            TaskEngineError::Integrity("pending audit has no report reservation".to_owned())
        })?;
        let plan = self.staged_plan(&prior_identity(&outcome.attempt), reservation)?;
        let receipt: ArtifactCommitReceipt = serde_json::from_value(outcome.receipt_json.clone())?;
        let payload = SanitizedPayload::try_from(json!({
            "status": outcome.status,
            "manifest_path": receipt.manifest_path,
            "manifest_revision": receipt.manifest_revision,
            "artifact_count": receipt.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id,
                    AuditPhase::Outcome,
                    "task_engine.report",
                    payload,
                )
                .with_subject(
                    AuditSubject::default()
                        .with_task(outcome.attempt.task_id)
                        .with_attempt(outcome.attempt.attempt_id),
                ),
            )
            .map_err(map_audit)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedReport {
    event_id: AuditEventId,
    report: RunnerReport,
}

pub(super) struct CommitPlan {
    // 字段放宽到 crate 内可见：terminal_staging 与 board::stopped 直接构造或读取计划。
    pub(super) event_id: AuditEventId,
    pub(super) artifacts: Vec<ArtifactCandidate>,
}

#[derive(Clone, Copy)]
pub(super) struct AttemptIdentity {
    // 字段放宽到 crate 内可见：terminal_staging 与 board::stopped 直接读取 attempt 身份。
    pub(super) task_id: TaskId,
    pub(super) attempt_id: AttemptId,
    pub(super) fence: Fence,
}

pub(super) fn prior_identity(attempt: &AttemptRecord) -> AttemptIdentity {
    AttemptIdentity {
        task_id: attempt.task_id,
        attempt_id: attempt.attempt_id,
        fence: attempt.fence,
    }
}

pub(super) fn decode_plan(
    attempt: &AttemptIdentity,
    reservation: &TerminalReservation,
) -> Result<CommitPlan, TaskEngineError> {
    let persisted: PersistedReport = serde_json::from_value(reservation.report_json.clone())?;
    let artifacts: Vec<ArtifactCandidate> =
        serde_json::from_value(reservation.artifacts_json.clone())?;
    if persisted.report.task_id != attempt.task_id
        || persisted.report.attempt_id != attempt.attempt_id
        || persisted.report.fence != attempt.fence
    {
        return Err(TaskEngineError::Integrity(
            "reserved report identity does not match attempt".to_owned(),
        ));
    }
    let declared: Vec<_> = persisted
        .report
        .artifacts
        .iter()
        .map(|artifact| ArtifactCandidate {
            name: artifact.name.clone(),
            content: artifact.content.clone(),
        })
        .collect();
    if declared != artifacts {
        return Err(TaskEngineError::Integrity(
            "reserved artifacts do not match runner report".to_owned(),
        ));
    }
    Ok(CommitPlan {
        event_id: persisted.event_id,
        artifacts,
    })
}
