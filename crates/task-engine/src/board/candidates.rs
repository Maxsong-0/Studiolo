//! runner 候选沿用现有 T1/T2 预留，再等待主人单独决定。
use super::*;
use crate::board_types::{contains_conflict_markers, sha256_hex};
use studiolo_runner_adapter::PatchMetadata;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 只发布 runner 补丁候选并进入 NeedsReview；绝不意味着已被接受。
    pub fn stage_board_runner_candidates(
        &mut self,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        self.verify_vault_binding()?;
        let task = self
            .task(report.task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        if self
            .control_db
            .board_task(report.task_id)
            .map_err(map_db)?
            .is_none()
            || task.spec.adapter == "consumer"
        {
            return Err(reject("only a board runner can submit review candidates"));
        }
        let attempt = self
            .control_db
            .attempt(report.task_id, report.attempt_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("attempt"))?;
        if attempt.fence != report.fence
            || task.active_attempt.or(task.last_attempt) != Some(report.attempt_id)
        {
            return Err(TaskEngineError::StaleFence);
        }
        let request = (
            report.task_id,
            report.attempt_id,
            report.fence,
            sha256_hex(&serde_json::to_vec(&report)?),
            now_ms,
        );
        self.audit_board("task.runner_candidates", &request, AuditPhase::Intent, None)?;
        let result = self.stage_board_runner_candidates_inner(&report, &attempt, now_ms);
        self.deliver_runtime_audits()?;
        self.audit_board(
            "task.runner_candidates",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    fn stage_board_runner_candidates_inner(
        &mut self,
        report: &RunnerReport,
        prior: &AttemptRecord,
        now: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        let event = AuditEventId::parse(format!(
            "runner-candidates:{}:{}",
            report.attempt_id,
            report.fence.get()
        ))
        .map_err(map_audit)?;
        let reservation = self.stage_reservation(&event, report)?;
        self.control_db
            .retain_runner_candidate(
                report.task_id,
                report.attempt_id,
                report.fence,
                &reservation,
                "retained",
            )
            .map_err(map_db)?;
        if let Err(error) = validate_candidates(report) {
            self.control_db
                .retain_runner_candidate(
                    report.task_id,
                    report.attempt_id,
                    report.fence,
                    &reservation,
                    "rejected",
                )
                .map_err(map_db)?;
            return Err(error);
        }
        match prior.phase {
            AttemptPhase::Running => {
                if !self
                    .control_db
                    .is_attempt_active_at(report.task_id, report.attempt_id, report.fence, now)
                    .map_err(map_db)?
                {
                    self.control_db
                        .retain_runner_candidate(
                            report.task_id,
                            report.attempt_id,
                            report.fence,
                            &reservation,
                            "lease_expired",
                        )
                        .map_err(map_db)?;
                    return Err(TaskEngineError::StaleFence);
                }
                self.control_db
                    .reserve_board_runner_candidates(
                        report.task_id,
                        report.attempt_id,
                        report.fence,
                        &reservation,
                        now,
                    )
                    .map_err(map_db)?;
            }
            AttemptPhase::Committing | AttemptPhase::Completed => {
                if prior.terminal.as_ref() != Some(&reservation) {
                    return Err(TaskEngineError::IdempotencyMismatch);
                }
                if prior.phase == AttemptPhase::Completed {
                    self.deliver_pending_outcomes(Some(prior.attempt_id), now)?;
                    return serde_json::from_value(
                        self.control_db
                            .artifact_receipt(prior.task_id, prior.attempt_id)
                            .map_err(map_db)?
                            .ok_or(TaskEngineError::NotFound("candidate receipt"))?,
                    )
                    .map_err(TaskEngineError::from);
                }
            }
            _ => {
                self.control_db
                    .retain_runner_candidate(
                        report.task_id,
                        report.attempt_id,
                        report.fence,
                        &reservation,
                        "cancelled_or_inactive",
                    )
                    .map_err(map_db)?;
                return Err(TaskEngineError::AlreadyTerminal);
            }
        }
        self.roll_forward(&prior_identity(prior), &reservation, now)
    }

    /// 受信任的 L3 主人入口：核验确切发布的候选，并记录明确的审阅结论。
    /// 组合根绝不能经由 AI 的黑板回执端点暴露此方法。
    pub fn review_board_candidates(
        &mut self,
        task: TaskId,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<BoardView, TaskEngineError> {
        let request = (task, &receipt, now);
        self.audit_board(
            "task.candidates_reviewed",
            &request,
            AuditPhase::Intent,
            None,
        )?;
        let result = self.review_board_candidates_inner(task, receipt.clone(), now);
        self.audit_board(
            "task.candidates_reviewed",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    fn review_board_candidates_inner(
        &mut self,
        task: TaskId,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<BoardView, TaskEngineError> {
        bounded(&receipt)?;
        let view = self
            .board_detail(task, "")?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        if view.task.spec.adapter == "consumer" {
            return Err(reject("consumer has no runner review candidates"));
        }
        let attempt = view
            .attempt
            .ok_or(TaskEngineError::NotFound("candidate attempt"))?;
        if view.task.status != TaskStatus::NeedsReview && view.board.receipt.is_none() {
            return Err(reject("candidates are not awaiting owner review"));
        }
        super::handoff::validate_receipt(&receipt, &view.board.definition.acceptance_criteria)?;
        let committed: ArtifactCommitReceipt = serde_json::from_value(
            self.control_db
                .artifact_receipt(task, attempt.attempt_id)
                .map_err(map_db)?
                .ok_or(TaskEngineError::NotFound("candidate receipt"))?,
        )?;
        let mut expected = committed
            .artifacts
            .iter()
            .map(|a| ArtifactPointer {
                path: a.path.clone(),
                revision: a.revision.to_string(),
                sha256: a.sha256.clone(),
            })
            .collect::<Vec<_>>();
        let mut actual = receipt.artifacts.clone();
        expected.sort_by(|a, b| a.path.cmp(&b.path));
        actual.sort_by(|a, b| a.path.cmp(&b.path));
        if actual != expected {
            return Err(reject(
                "owner review must include every exact committed candidate pointer",
            ));
        }
        if view.board.receipt.is_none() {
            for pointer in receipt
                .artifacts
                .iter()
                .chain(receipt.acceptance.iter().flat_map(|a| a.evidence.iter()))
            {
                self.verify_artifact_pointer(pointer)?;
            }
            for pointer in &receipt.unlanded {
                let object = self
                    .broker
                    .read(&pointer.path)
                    .map_err(TaskEngineError::Broker)?;
                if sha256_hex(&object.content) != pointer.sha256.to_ascii_lowercase() {
                    return Err(reject("unlanded patch pointer mismatch"));
                }
            }
        }
        let status = match receipt.status {
            ReceiptStatus::Completed => TaskStatus::Succeeded,
            ReceiptStatus::Failed => TaskStatus::Failed,
            ReceiptStatus::Stopped => TaskStatus::Cancelled,
            ReceiptStatus::Abandoned => TaskStatus::Abandoned,
        };
        self.control_db
            .review_board_candidates(
                task,
                attempt.attempt_id,
                attempt.fence,
                &serde_json::to_value(receipt)?,
                status,
                now,
            )
            .map_err(map_db)?;
        self.board_detail(task, &view.board.definition.policy_hash)?
            .ok_or(TaskEngineError::NotFound("board task"))
    }
}

pub(super) fn validate_candidates(report: &RunnerReport) -> Result<(), TaskEngineError> {
    if report.artifacts.len() != 2 {
        return Err(reject(
            "runner candidates require candidate.patch and patch.json only",
        ));
    }
    let patch = report
        .artifacts
        .iter()
        .find(|a| a.name.as_str() == "candidate.patch")
        .ok_or_else(|| reject("missing candidate.patch"))?;
    let metadata = report
        .artifacts
        .iter()
        .find(|a| a.name.as_str() == "patch.json")
        .ok_or_else(|| reject("missing patch.json"))?;
    let metadata: PatchMetadata = serde_json::from_slice(&metadata.content)?;
    let tree = |id: &str| matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit());
    if !tree(&metadata.base_tree)
        || !tree(&metadata.after_tree)
        || metadata.sha256 != sha256_hex(&patch.content)
    {
        return Err(reject("runner patch metadata does not match candidate"));
    }
    if contains_conflict_markers(&patch.content) {
        return Err(reject(
            "runner candidate contains unresolved conflict markers",
        ));
    }
    Ok(())
}
