//! 显式启动恢复与周期租约对账的撤销规则不同。
use super::*;
use studiolo_control_db::LeaseReconciliation;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 续期租约，同时上报单调进度。
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

    /// 周期对账绝不放弃健康且有进展的活动 attempt。
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
                    // 单个被保留的发布不能挡住其他租约的处理；它的预留仍是
                    // Committing，因此新持有者和成功回执都无法胜出。
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

    /// 仅用于启动的黑板撤销，须在组合根隔离旧 runner 之后调用。
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

    /// 结构化的工具调用前启动失败可以重试；任务本身的失败从不重试。
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
