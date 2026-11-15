//! 内联的 v1/v2 预留先经 Broker 持久化，再从数据库中移除正文。
use super::*;
use studiolo_vault_broker::StagedArtifactCommit;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    pub(super) fn stage_reservation(
        &mut self,
        event: &AuditEventId,
        report: &RunnerReport,
    ) -> Result<TerminalReservation, TaskEngineError> {
        let provenance = self.bound_provenance(&AttemptIdentity {
            task_id: report.task_id,
            attempt_id: report.attempt_id,
            fence: report.fence,
        })?;
        let commit = ArtifactCommit {
            task_id: report.task_id,
            attempt_id: report.attempt_id,
            fence: report.fence,
            audit_intent: event.to_string(),
            idempotency_key: format!("task-engine:{}:{}", report.attempt_id, event),
            artifacts: report
                .artifacts
                .iter()
                .map(|a| ArtifactCandidate {
                    name: a.name.clone(),
                    content: a.content.clone(),
                })
                .collect(),
            provenance,
        };
        let pointer = self
            .broker
            .stage_artifact_commit(&commit)
            .map_err(TaskEngineError::Broker)?;
        pointer_reservation(&commit, &pointer)
    }

    pub(super) fn staged_plan(
        &self,
        attempt: &AttemptIdentity,
        reservation: &TerminalReservation,
    ) -> Result<CommitPlan, TaskEngineError> {
        if reservation.artifacts_json.is_array() {
            return decode_plan(attempt, reservation);
        }
        let pointer: StagedArtifactCommit =
            serde_json::from_value(reservation.artifacts_json.clone())?;
        let commit = self
            .broker
            .load_staged_artifact_commit(&pointer)
            .map_err(TaskEngineError::Broker)?;
        let mut metadata = serde_json::json!({"format":3,"event_id":commit.audit_intent,"task_id":commit.task_id,"attempt_id":commit.attempt_id,"fence":commit.fence});
        if reservation
            .report_json
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            == Some("stopped")
        {
            metadata["outcome"] = serde_json::json!("stopped");
        }
        if commit.task_id != attempt.task_id
            || commit.attempt_id != attempt.attempt_id
            || commit.fence != attempt.fence
            || reservation.report_json != metadata
        {
            return Err(TaskEngineError::Integrity(
                "staged artifact identity differs".into(),
            ));
        }
        Ok(CommitPlan {
            event_id: AuditEventId::parse(commit.audit_intent).map_err(map_audit)?,
            artifacts: commit.artifacts,
            staged_provenance: Some(commit.provenance),
        })
    }

    /// 来源只取原批准和原账本。报告里没有授权字段，旧 attempt 缺记录时不补完整范围。
    pub(super) fn bound_provenance(
        &mut self,
        attempt: &AttemptIdentity,
    ) -> Result<ArtifactProvenance, TaskEngineError> {
        self.deliver_runtime_audits()?;
        let stored = self
            .control_db
            .artifact_provenance(attempt.task_id, attempt.attempt_id, attempt.fence)
            .map_err(map_db)?
            .ok_or_else(|| {
                TaskEngineError::Integrity("旧 attempt 没有产物批准，不能补成完整范围".into())
            })?;
        let expected = self
            .control_db
            .artifact_approval_event(attempt.task_id, attempt.attempt_id, attempt.fence)
            .map_err(map_db)?
            .ok_or_else(|| TaskEngineError::Integrity("原账本缺少产物批准事实".into()))?;
        let mut records = self
            .audit_ledger
            .query(
                &AuditFilter::default()
                    .with_event_id(expected.event_id().clone())
                    .with_limit(2),
            )
            .map_err(map_audit)?;
        let Some(record) = records.pop() else {
            return Err(TaskEngineError::Integrity(
                "原账本的产物批准不是唯一事实".into(),
            ));
        };
        if !records.is_empty() {
            return Err(TaskEngineError::Integrity(
                "原账本的产物批准不是唯一事实".into(),
            ));
        }
        record.verify_integrity().map_err(map_audit)?;
        if record.event() != &expected {
            return Err(TaskEngineError::Integrity("原账本与原批准不一致".into()));
        }
        Ok(stored)
    }

    /// 返回已安全迁入内容面 staging 的旧预留数量。
    pub fn migrate_terminal_bodies(&mut self) -> Result<usize, TaskEngineError> {
        let attempts = self.control_db.legacy_terminal_attempts().map_err(map_db)?;
        for attempt in &attempts {
            let prior = attempt
                .terminal
                .as_ref()
                .ok_or_else(|| TaskEngineError::Integrity("legacy reservation missing".into()))?;
            let identity = prior_identity(attempt);
            let plan = decode_plan(&identity, prior)?;
            if plan.staged_provenance.is_some() {
                return Err(TaskEngineError::Integrity(
                    "旧内联预留不能自带完整范围".into(),
                ));
            }
            let commit = ArtifactCommit {
                task_id: attempt.task_id,
                attempt_id: attempt.attempt_id,
                fence: attempt.fence,
                audit_intent: plan.event_id.to_string(),
                idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
                artifacts: plan.artifacts,
                provenance: self.bound_provenance(&identity)?,
            };
            let pointer = self
                .broker
                .stage_artifact_commit(&commit)
                .map_err(TaskEngineError::Broker)?;
            self.control_db
                .migrate_terminal_pointer(
                    attempt.attempt_id,
                    prior,
                    &pointer_reservation(&commit, &pointer)?,
                )
                .map_err(map_db)?;
        }
        Ok(attempts.len())
    }
}

fn pointer_reservation(
    commit: &ArtifactCommit,
    pointer: &StagedArtifactCommit,
) -> Result<TerminalReservation, TaskEngineError> {
    Ok(TerminalReservation {
        report_json: serde_json::json!({"format":3,"event_id":commit.audit_intent,"task_id":commit.task_id,"attempt_id":commit.attempt_id,"fence":commit.fence}),
        artifacts_json: serde_json::to_value(pointer)?,
    })
}
