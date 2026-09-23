//! 取消时经同一 T1/T2 保留真实候选与 runner 的停止包。
use super::*;
use crate::board_types::sha256_hex;
use studiolo_runner_adapter::RunnerArtifact;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 在 180 秒宽限期内，随停止回执发布真实的取消候选。
    /// 已提交的取消绝不宣称被接受，也绝不恢复外部 runner。
    pub fn stage_board_stopped_candidates(
        &mut self,
        mut report: RunnerReport,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        self.verify_vault_binding()?;
        bounded(&receipt)?;
        let task = self
            .task(report.task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        let board = self
            .control_db
            .board_task(report.task_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        if task.spec.adapter == "consumer" {
            return Err(reject("consumer cannot submit stopped runner candidates"));
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
        let packets = report
            .artifacts
            .iter()
            .filter(|a| a.name.as_str() == "stopped.json")
            .collect::<Vec<_>>();
        if packets.len() > 1
            || packets.first().is_some_and(|a| {
                serde_json::from_slice::<BoardReceipt>(&a.content)
                    .ok()
                    .as_ref()
                    != Some(&receipt)
            })
        {
            return Err(reject("stopped.json differs from the supplied receipt"));
        }
        if packets.is_empty() {
            report.artifacts.push(RunnerArtifact {
                name: VaultPath::parse("stopped.json")
                    .map_err(|e| TaskEngineError::Integrity(e.to_string()))?,
                content: serde_json::to_vec(&receipt)?,
            });
        }
        let request = (
            report.task_id,
            report.attempt_id,
            report.fence,
            sha256_hex(&serde_json::to_vec(&report)?),
            now,
        );
        self.audit_board(
            "task.stopped_candidates",
            &request,
            AuditPhase::Intent,
            None,
        )?;
        let result = self.stage_stopped_inner(
            &report,
            &receipt,
            &attempt,
            &board.definition.acceptance_criteria,
            now,
        );
        self.deliver_runtime_audits()?;
        self.audit_board(
            "task.stopped_candidates",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    fn stage_stopped_inner(
        &mut self,
        report: &RunnerReport,
        receipt: &BoardReceipt,
        prior: &AttemptRecord,
        criteria: &[String],
        now: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        let event = AuditEventId::parse(format!(
            "runner-stopped:{}:{}",
            report.attempt_id,
            report.fence.get()
        ))
        .map_err(map_audit)?;
        let mut reservation = self.stage_reservation(&event, report)?;
        reservation.report_json["outcome"] = json!("stopped");
        self.retain_stopped(report, &reservation, "retained")?;
        let validation = validate_stopped(report, receipt, criteria);
        if let Err(error) = validation {
            self.retain_stopped(report, &reservation, "rejected")?;
            return Err(error);
        }
        match prior.phase {
            AttemptPhase::CancelRequested => {
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
                        return Err(reject("unlanded stopped pointer mismatch"));
                    }
                }
                if let Err(error) = self.control_db.reserve_board_stopped_candidates(
                    report.task_id,
                    report.attempt_id,
                    report.fence,
                    &reservation,
                    now,
                ) {
                    self.retain_stopped(report, &reservation, "cancelled_or_inactive")?;
                    return Err(map_db(error));
                }
            }
            AttemptPhase::Committing | AttemptPhase::Cancelled => {
                if prior.terminal.as_ref() != Some(&reservation) {
                    return Err(TaskEngineError::IdempotencyMismatch);
                }
                if prior.phase == AttemptPhase::Cancelled {
                    self.deliver_pending_outcomes(Some(prior.attempt_id), now)?;
                    return serde_json::from_value(
                        self.control_db
                            .artifact_receipt(prior.task_id, prior.attempt_id)
                            .map_err(map_db)?
                            .ok_or(TaskEngineError::NotFound("stopped artifacts"))?,
                    )
                    .map_err(TaskEngineError::from);
                }
            }
            _ => {
                self.retain_stopped(report, &reservation, "cancelled_or_inactive")?;
                return Err(TaskEngineError::AlreadyTerminal);
            }
        }
        self.roll_forward(&prior_identity(prior), &reservation, now)
    }

    fn retain_stopped(
        &mut self,
        report: &RunnerReport,
        reservation: &TerminalReservation,
        reason: &str,
    ) -> Result<(), TaskEngineError> {
        self.control_db
            .retain_runner_candidate(
                report.task_id,
                report.attempt_id,
                report.fence,
                reservation,
                reason,
            )
            .map_err(map_db)
    }

    pub(crate) fn stopped_receipt_from_plan(
        &self,
        attempt: &AttemptIdentity,
        plan: &CommitPlan,
    ) -> Result<BoardReceipt, TaskEngineError> {
        let board = self
            .control_db
            .board_task(attempt.task_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        let report = RunnerReport {
            task_id: attempt.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            artifacts: plan
                .artifacts
                .iter()
                .map(|a| RunnerArtifact {
                    name: a.name.clone(),
                    content: a.content.clone(),
                })
                .collect(),
        };
        let packet = report
            .artifacts
            .iter()
            .find(|a| a.name.as_str() == "stopped.json")
            .ok_or_else(|| reject("reserved stopped packet is missing"))?;
        let receipt: BoardReceipt = serde_json::from_slice(&packet.content)?;
        validate_stopped(&report, &receipt, &board.definition.acceptance_criteria)?;
        Ok(receipt)
    }

    pub(crate) fn finish_stopped_candidates(
        &mut self,
        attempt: &AttemptIdentity,
        mut stopped: BoardReceipt,
        committed: &ArtifactCommitReceipt,
        now: i64,
    ) -> Result<(), TaskEngineError> {
        for artifact in &committed.artifacts {
            let pointer = ArtifactPointer {
                path: artifact.path.clone(),
                revision: artifact.revision.to_string(),
                sha256: artifact.sha256.clone(),
            };
            self.verify_artifact_pointer(&pointer)?;
            if artifact.path.as_str().ends_with("/candidate.patch") {
                let unlanded = UnlandedPointer {
                    path: artifact.path.clone(),
                    sha256: artifact.sha256.clone(),
                };
                if !stopped.unlanded.contains(&unlanded) {
                    stopped.unlanded.push(unlanded);
                }
            }
            if !stopped.artifacts.contains(&pointer) {
                stopped.artifacts.push(pointer);
            }
        }
        bounded(&stopped)?;
        self.control_db
            .finalize_board_stopped_candidates(
                attempt.task_id,
                attempt.attempt_id,
                attempt.fence,
                &serde_json::to_value(committed)?,
                &serde_json::to_value(stopped)?,
                now,
            )
            .map_err(map_db)
    }
}

fn validate_stopped(
    report: &RunnerReport,
    receipt: &BoardReceipt,
    criteria: &[String],
) -> Result<(), TaskEngineError> {
    bounded(receipt)?;
    if serde_json::to_vec(receipt)?.len() > 32 * 1024
        || receipt.artifacts.len() > 253
        || receipt.unlanded.len() > 255
    {
        return Err(reject(
            "stopped receipt must leave room for committed candidate pointers",
        ));
    }
    super::handoff::validate_receipt(receipt, criteria)?;
    if receipt.status != ReceiptStatus::Stopped
        || receipt
            .acceptance
            .iter()
            .all(|a| a.result == AssessmentResult::Pass)
        || receipt
            .acceptance
            .iter()
            .any(|a| a.result == AssessmentResult::Pass && a.evidence.is_empty())
    {
        return Err(reject(
            "stopped receipt must preserve unfinished assessments and evidence for any pass",
        ));
    }
    if report.artifacts.len() != 3
        || report
            .artifacts
            .iter()
            .filter(|a| a.name.as_str() == "stopped.json")
            .count()
            != 1
    {
        return Err(reject(
            "stopped candidates require candidate.patch, patch.json, and stopped.json",
        ));
    }
    let mut candidates = report.clone();
    candidates
        .artifacts
        .retain(|a| a.name.as_str() != "stopped.json");
    super::candidates::validate_candidates(&candidates)
}
