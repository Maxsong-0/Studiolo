//! Audit is written before protected board effects. Replays share a stable request digest.
use super::*;
use crate::board_types::{contains_conflict_markers, sha256_hex};

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    pub(super) fn audit_board<T: Serialize>(
        &mut self,
        action: &str,
        request: &T,
        phase: AuditPhase,
        outcome: Option<bool>,
    ) -> Result<(), TaskEngineError> {
        if phase == AuditPhase::Outcome && outcome == Some(true) {
            self.deliver_runtime_audits()?;
        }
        let digest = sha256_hex(&serde_json::to_vec(request)?);
        let id = AuditEventId::parse(format!("board:{action}:{digest}")).map_err(map_audit)?;
        let payload = if let Some(success) = outcome {
            json!({"request_hash":digest,"status":if success {"succeeded"}else{"rejected"}})
        } else {
            json!({"request_hash":digest})
        };
        self.audit_ledger
            .append(AuditEvent::new(
                id,
                phase,
                action,
                SanitizedPayload::try_from(payload).map_err(map_audit)?,
            ))
            .map_err(map_audit)?;
        Ok(())
    }

    /// Repairs metadata-only authority outcomes after ledger outages or process restarts.
    pub fn deliver_runtime_audits(&mut self) -> Result<usize, TaskEngineError> {
        let events = self.control_db.pending_runtime_audits().map_err(map_db)?;
        for event in &events {
            let mut subject = AuditSubject::default();
            if let Some(task) = event.payload.get("task_id") {
                subject = subject.with_task(serde_json::from_value(task.clone())?);
            }
            if let Some(attempt) = event.payload.get("attempt_id") {
                subject = subject.with_attempt(serde_json::from_value(attempt.clone())?);
            }
            self.audit_ledger
                .append(
                    AuditEvent::new(
                        AuditEventId::parse(format!("runtime:{}", event.event_id))
                            .map_err(map_audit)?,
                        AuditPhase::Outcome,
                        &event.action,
                        SanitizedPayload::try_from(event.payload.clone()).map_err(map_audit)?,
                    )
                    .with_subject(subject),
                )
                .map_err(map_audit)?;
            self.control_db
                .acknowledge_runtime_audit(&event.event_id)
                .map_err(map_db)?;
        }
        Ok(events.len())
    }

    pub(super) fn verify_artifact_pointer(
        &self,
        pointer: &ArtifactPointer,
    ) -> Result<(), TaskEngineError> {
        let object = self
            .broker
            .read(&pointer.path)
            .map_err(TaskEngineError::Broker)?;
        if object.revision.to_string() != pointer.revision
            || sha256_hex(&object.content) != pointer.sha256.to_ascii_lowercase()
        {
            return Err(TaskEngineError::Integrity(
                "artifact pointer does not match content-plane bytes".into(),
            ));
        }
        if contains_conflict_markers(&object.content) {
            return Err(TaskEngineError::InvalidTaskSpec(
                "artifact contains unresolved conflict markers",
            ));
        }
        Ok(())
    }
}
