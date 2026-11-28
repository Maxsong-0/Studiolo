//! 显式启动恢复与周期租约对账的撤销规则不同。
use super::*;
use crate::board_audit::BoardVerdict;
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
        let audit = self.board_intent("task.heartbeat", &request)?;
        let result = self
            .control_db
            .heartbeat_progress(task, attempt, fence, lease_until, tool_calls, edits, now)
            .map_err(map_db);
        self.board_outcome(audit, BoardVerdict::of(&result))?;
        result
    }

    /// 周期对账绝不放弃健康且有进展的活动 attempt。
    pub fn reconcile_periodic(&mut self, now: i64) -> Result<LeaseReconciliation, TaskEngineError> {
        self.verify_vault_binding()?;
        self.migrate_terminal_bodies()?;
        // 账本中断后没投出去的运行时结果每轮补投一次。以前它顺带在每轮的 `task.reconcile` 结果审计里做，
        // 没事可做的轮次不再记审计之后（T-213）要单独调，补投节奏不变。
        self.deliver_runtime_audits()?;
        let report = self.reconcile_leases_audited(now, false)?;
        self.deliver_pending_outcomes(None, now)?;
        for attempt in self.control_db.committing_attempts().map_err(map_db)? {
            let result = match attempt.terminal.as_ref() {
                Some(reservation) => self
                    .roll_forward(&prior_identity(&attempt), reservation, now)
                    .map(|_| ()),
                None => Err(TaskEngineError::Integrity(
                    "committing attempt has no reservation".into(),
                )),
            };
            match result {
                Ok(()) => {}
                Err(error) if super::terminal_staging::reservation_specific(&error) => {
                    // 单个被保留的发布、或者一份坏掉的预留（旧内联格式缺原批准、内容对不上，复审 D3-F5）
                    // 不能挡住其他租约的处理；它的预留仍是 Committing，因此新持有者和成功回执都无法胜出。
                    // 原批准过期、一篇正文都没写出去的提交也停在这里、保留暂存，对账不替主人丢：
                    // 只有主人经 `discard_expired_commit` 决定（2026-11-16 用户拍板）。
                    // 同一个 attempt 只记一次（请求里不带时间，账本按幂等键重放）：主人几天不处理时，
                    // 每 15 秒一条会让审计无界增长（复审 A1-F4）。
                    self.board_pending_marker(
                        "task.candidate_recovery_pending",
                        &(attempt.task_id, attempt.attempt_id),
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
        let report = self.reconcile_leases_audited(now, true)?;
        self.reconcile_periodic(now)?;
        Ok(report)
    }

    /// 先预演、有事可做才记意图再照单执行（T-213）。意图仍先于动作落账，请求里带这一轮的整张清单，
    /// 所以账本里的意图覆盖了这一轮做的每一步；没事可做的轮次不动控制库，也不记审计，
    /// 常驻每 15 秒一轮不再让账本空转增长。
    fn reconcile_leases_audited(
        &mut self,
        now: i64,
        startup: bool,
    ) -> Result<LeaseReconciliation, TaskEngineError> {
        let plan = self
            .control_db
            .plan_lease_reconciliation(now, startup)
            .map_err(map_db)?;
        if plan.is_empty() {
            return Ok(LeaseReconciliation::default());
        }
        let request = (now, startup, &plan);
        let audit = self.board_intent("task.reconcile", &request)?;
        let applied = self
            .control_db
            .apply_lease_plan(now, startup, &plan)
            .map_err(map_db);
        // 预演之后库变了的那一轮什么都没做（结果记成没成），下一轮重新预演；控制库忙这类没下结论的不记结果。
        self.board_outcome(audit, BoardVerdict::of_plan(&applied))?;
        Ok(applied?.unwrap_or_default())
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
        let audit = self.board_intent("task.launch_failed", &(task, attempt, fence, now))?;
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
        self.board_outcome(audit, BoardVerdict::of(&result))?;
        result
    }
}
