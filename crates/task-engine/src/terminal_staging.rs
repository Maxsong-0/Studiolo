//! Inline v1/v2 reservations migrate through Broker durability before their DB body is removed.
use super::*;
use studiolo_vault_broker::StagedArtifactCommit;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    pub(super) fn stage_reservation(
        &self,
        event: &AuditEventId,
        report: &RunnerReport,
    ) -> Result<TerminalReservation, TaskEngineError> {
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
        })
    }

    /// Returns the number of legacy reservations safely moved to content-plane staging.
    pub fn migrate_terminal_bodies(&mut self) -> Result<usize, TaskEngineError> {
        let attempts = self.control_db.legacy_terminal_attempts().map_err(map_db)?;
        for attempt in &attempts {
            let prior = attempt
                .terminal
                .as_ref()
                .ok_or_else(|| TaskEngineError::Integrity("legacy reservation missing".into()))?;
            let plan = decode_plan(&prior_identity(attempt), prior)?;
            let commit = ArtifactCommit {
                task_id: attempt.task_id,
                attempt_id: attempt.attempt_id,
                fence: attempt.fence,
                audit_intent: plan.event_id.to_string(),
                idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
                artifacts: plan.artifacts,
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
