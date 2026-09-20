//! Explicit startup recovery and periodic lease reconciliation have different revocation rules.
use super::*;
use studiolo_control_db::LeaseReconciliation;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// Extends a lease while reporting monotonic progress.
    #[allow(clippy::too_many_arguments)]
    pub fn heartbeat_progress(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        lease_until: i64,
        tool_calls: u64,
        edits: u64,
        now: i64,
    ) -> Result<(), TaskEngineError> {
        let request = (task, attempt, fence, lease_until, tool_calls, edits, now);
        self.audit_board("task.heartbeat", &request, AuditPhase::Intent, None)?;
        let result = self
            .control_db
            .heartbeat_progress(task, attempt, fence, lease_until, tool_calls, edits, now)
            .map_err(map_db);
        self.audit_board(
            "task.heartbeat",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    /// Periodic reconciliation never abandons a healthy, progressing live attempt.
    pub fn reconcile_periodic(&mut self, now: i64) -> Result<LeaseReconciliation, TaskEngineError> {
        self.verify_vault_binding()?;
        self.migrate_terminal_bodies()?;
        self.audit_board("task.reconcile", &(now, false), AuditPhase::Intent, None)?;
        let report = self
            .control_db
            .reconcile_leases(now, false)
            .map_err(map_db)?;
        self.audit_board(
            "task.reconcile",
            &(now, false),
            AuditPhase::Outcome,
            Some(true),
        )?;
        self.deliver_pending_outcomes(None, now)?;
        for attempt in self.control_db.committing_attempts().map_err(map_db)? {
            let reservation = attempt.terminal.as_ref().ok_or_else(|| {
                TaskEngineError::Integrity("committing attempt has no reservation".into())
            })?;
            match self.roll_forward(&prior_identity(&attempt), reservation, now) {
                Ok(_) => {}
                Err(TaskEngineError::CommitFailed(_)) => {
                    // A single retained publication cannot stop other leases; its reservation
                    // remains Committing, so neither a new holder nor successful receipt can win.
                    self.audit_board(
                        "task.candidate_recovery_pending",
                        &(attempt.task_id, attempt.attempt_id, now),
                        AuditPhase::Outcome,
                        Some(false),
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(report)
    }

    /// Startup-only board revocation after the composition root has isolated old runners.
    pub fn reconcile_startup(&mut self, now: i64) -> Result<LeaseReconciliation, TaskEngineError> {
        self.verify_vault_binding()?;
        self.migrate_terminal_bodies()?;
        self.audit_board("task.reconcile", &(now, true), AuditPhase::Intent, None)?;
        let report = self
            .control_db
            .reconcile_leases(now, true)
            .map_err(map_db)?;
        self.audit_board(
            "task.reconcile",
            &(now, true),
            AuditPhase::Outcome,
            Some(true),
        )?;
        self.reconcile_periodic(now)?;
        Ok(report)
    }

    /// A structured pre-tool launch failure may retry; task failures never do.
    pub fn launch_failed(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        now: i64,
    ) -> Result<bool, TaskEngineError> {
        let spec = self
            .task(task)?
            .ok_or(TaskEngineError::NotFound("task"))?
            .spec;
        self.audit_board(
            "task.launch_failed",
            &(task, attempt, fence, now),
            AuditPhase::Intent,
            None,
        )?;
        let result = self
            .control_db
            .launch_failed(
                task,
                attempt,
                fence,
                spec.retry_policy
                    .retry_on
                    .iter()
                    .any(|value| value == "launch_failure"),
                spec.retry_policy.max_attempts,
                i64::try_from(spec.retry_policy.backoff_seconds.saturating_mul(1000))
                    .unwrap_or(i64::MAX),
                now,
            )
            .map_err(map_db);
        self.audit_board(
            "task.launch_failed",
            &(task, attempt, fence, now),
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
}
