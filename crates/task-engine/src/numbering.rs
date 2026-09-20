//! Namespace allocation always observes real Broker filenames, never client-supplied maxima.
use super::*;
use studiolo_vault_broker::NumberingScanner;
impl<B: VaultBroker + ArtifactStager + NumberingScanner, L: AuditLedger>
    PersistentTaskEngine<B, L>
{
    /// Scans canonical filenames then atomically reserves the next number with stable replay.
    pub fn reserve_number(
        &mut self,
        request_id: &str,
        task: TaskId,
        scope: &VaultPath,
        prefix: &str,
    ) -> Result<u64, TaskEngineError> {
        if request_id.is_empty()
            || request_id.len() > 256
            || prefix.is_empty()
            || prefix.len() > 64
            || prefix.contains('/')
        {
            return Err(TaskEngineError::InvalidTaskSpec(
                "invalid numbering request",
            ));
        }
        if self.control_db.board_task(task).map_err(map_db)?.is_none() {
            return Err(TaskEngineError::NotFound("board task"));
        }
        let request = (request_id, task, scope, prefix);
        self.audit_board("task.number_reserved", &request, AuditPhase::Intent, None)?;
        let maximum = self
            .broker
            .scan_number_max(scope, prefix)
            .map_err(TaskEngineError::Broker)?;
        let result = self
            .control_db
            .reserve_board_number(
                request_id,
                task,
                &format!("{}:{}", scope.collision_key(), prefix.to_lowercase()),
                maximum,
            )
            .map_err(map_db);
        self.audit_board(
            "task.number_reserved",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
}
