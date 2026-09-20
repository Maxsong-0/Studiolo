//! Per-attempt maintenance: a failed candidate never interrupts other supervised runs.
use super::{ActiveRun, RunnerPlane};
use std::sync::atomic::Ordering;
use studiolo_control_db::AttemptPhase;
use studiolo_runner_adapter::{RunnerAdapter, RunnerObservation};
use studiolo_task_engine::{TaskCommand, TaskEngine};

impl RunnerPlane {
    /// Rechecks leases/fences/revocation and stages candidates independently per attempt.
    /// Call in spawn_blocking at most once per maintenance tick, outside the engine lock.
    pub fn tick(&self) -> Result<(), String> {
        let mut runs = self.runs.lock().map_err(|_| "Runner状态不可用")?;
        let now = jiff::Timestamp::now().as_millisecond();
        for run in runs.values_mut() {
            if run.cleanup_pending {
                run.cleanup_pending = self.stop_and_close(run, now).is_err();
                continue;
            }
            if run.summary.state != "running" {
                continue;
            }
            if self.advance(run, now).is_err() {
                if run.summary.state == "running" {
                    run.summary.state = "failed".into();
                }
                // A retained patch and any engine reservation remain available for recovery.
                // No error here restarts a CLI or prevents the next attempt being supervised.
                run.cleanup_pending = self.stop_and_close(run, now).is_err();
            }
        }
        Ok(())
    }

    fn advance(&self, run: &mut ActiveRun, now: i64) -> Result<(), String> {
        let state = self
            .authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?
            .control_db()
            .attempt(run.spec.task_id, run.spec.attempt_id)
            .map_err(|_| "运行状态不可用")?
            .ok_or("attempt缺失")?;
        let grant = self
            .authority
            .bindings
            .lock()
            .map_err(|_| "Runner授权不可用")?
            .get(&run.spec.attempt_id)
            .and_then(|b| b.grant.clone());
        let authorized = grant.as_ref().is_some_and(|grant| {
            studiolo_gateway::RelayPolicyGuard::inspect(self.authority.as_ref(), grant, now / 1000)
                .is_ok()
        });
        let stopping = self.authority.frozen.load(Ordering::SeqCst)
            || state.phase != AttemptPhase::Running
            || !authorized;
        if stopping {
            self.revoke_ticket(run.spec.attempt_id, &run.ticket)?;
            let _ = self
                .authority
                .board
                .engine
                .lock()
                .map_err(|_| "黑板不可用")?
                .command(run.spec.task_id, TaskCommand::Cancel, now);
            run.runner
                .cancel(&run.handle)
                .map_err(|_| "执行体取消失败")?;
        }
        match run.runner.observe(&run.handle) {
            Ok(RunnerObservation::Running) => {
                run.summary.progress = run
                    .runner
                    .progress(&run.handle)
                    .map_err(|_| "执行体进度不可用")?;
                if run.summary.progress.phase == studiolo_runner_adapter::BudgetPhase::Exhausted {
                    self.revoke_ticket(run.spec.attempt_id, &run.ticket)?;
                    let _ = self
                        .authority
                        .board
                        .engine
                        .lock()
                        .map_err(|_| "黑板不可用")?
                        .command(run.spec.task_id, TaskCommand::Cancel, now);
                    return Ok(());
                }
                // Cancellation and revocation are never extended by a heartbeat.
                if !stopping && !self.authority.frozen.load(Ordering::SeqCst) {
                    self.authority
                        .board
                        .engine
                        .lock()
                        .map_err(|_| "黑板不可用")?
                        .heartbeat_progress(
                            run.spec.task_id,
                            run.spec.attempt_id,
                            run.spec.fence,
                            now.saturating_add(120_000),
                            run.summary.progress.tool_calls,
                            run.summary.progress.edits,
                            now,
                        )
                        .map_err(|_| "执行体心跳拒绝")?;
                }
            }
            Ok(RunnerObservation::Completed(report)) => {
                // RealRunner preserved the candidate outside scratch before returning this report.
                run.summary.state = "candidate_retained".into();
                self.revoke_ticket(run.spec.attempt_id, &run.ticket)?;
                super::recovery::isolate_unit(&unit(run))?;
                self.authority
                    .board
                    .engine
                    .lock()
                    .map_err(|_| "黑板不可用")?
                    .stage_board_runner_candidates(report, now)
                    .map_err(|_| "候选产物已留存，封存待恢复；未验收完成")?;
                run.summary.state = "candidate_review".into();
            }
            Ok(RunnerObservation::Stopped { report, receipt }) => {
                self.revoke_ticket(run.spec.attempt_id, &run.ticket)?;
                super::recovery::isolate_unit(&unit(run))?;
                let mut engine = self
                    .authority
                    .board
                    .engine
                    .lock()
                    .map_err(|_| "黑板不可用")?;
                // Budget-triggered cancellation can be first observed together with process exit.
                let _ = engine.command(run.spec.task_id, TaskCommand::Cancel, now);
                run.summary.state = "candidate_retained".into();
                if let Ok(receipt) = serde_json::from_value(receipt)
                    && engine
                        .stage_board_stopped_candidates(report, receipt, now)
                        .is_ok()
                {
                    run.summary.state = "stopped".into();
                }
                // Invalid or late packages remain retained; only T1 recovery or the original
                // cancellation deadline may close the claim, never an invented receipt.
            }
            Ok(RunnerObservation::Cancelled) => {
                run.summary.state = "stopped".into();
                self.stop_and_close(run, now)?;
            }
            Err(error) => {
                let observed = run
                    .runner
                    .progress(&run.handle)
                    .ok()
                    .map(|p| p.tool_calls.max(run.summary.progress.tool_calls));
                let persisted = self
                    .authority
                    .board
                    .engine
                    .lock()
                    .ok()
                    .and_then(|engine| {
                        engine
                            .control_db()
                            .attempt_progress(run.spec.attempt_id)
                            .ok()
                            .flatten()
                    })
                    .map(|p| p.tool_calls);
                run.launch_failed = launch_failure(&error, stopping, observed, persisted);
                return Err("执行体失败，候选工作区保留".into());
            }
        }
        Ok(())
    }

    /// Removes the live grant before unlinking its short-lived ticket. Attempts remain revoked
    /// even if unlink fails; the maintenance retry never treats a leftover file as authority.
    pub(super) fn revoke_ticket(
        &self,
        attempt: studiolo_core::AttemptId,
        path: &std::path::Path,
    ) -> Result<(), String> {
        match self.authority.bindings.lock() {
            Ok(mut bindings) => {
                bindings.remove(&attempt);
            }
            Err(_) => {
                self.admission.invalidate();
                self.authority.frozen.store(true, Ordering::SeqCst);
                let _ = self.relay.set_frozen(true);
                return Err("Runner授权状态不可用，已冻结".into());
            }
        }
        match std::fs::remove_file(path) {
            Ok(()) => std::fs::File::open(&self.authority.config.state_root)
                .and_then(|f| f.sync_all())
                .map_err(|_| "短期票据目录持久化失败".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("已撤权票据清理待重试".into()),
        }
    }

    pub(super) fn stop_and_close(&self, run: &ActiveRun, now: i64) -> Result<(), String> {
        // Execute all independent cleanup steps even if one fails.
        let revoked = self.revoke_ticket(run.spec.attempt_id, &run.ticket);
        let cancelled = if run.launch_failed {
            Ok(())
        } else {
            self.close_claim(run, now, false)
        };
        let isolated = super::recovery::isolate_unit(&unit(run));
        let closed = if isolated.is_ok() && revoked.is_ok() {
            self.close_claim(run, now, true)
        } else {
            Ok(())
        };
        revoked.and(cancelled).and(isolated).and(closed)
    }

    fn close_claim(&self, run: &ActiveRun, now: i64, isolated: bool) -> Result<(), String> {
        let mut engine = self
            .authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?;
        let task = engine
            .task(run.spec.task_id)
            .map_err(|_| "任务不可用")?
            .ok_or("任务缺失")?;
        if task.active_attempt != Some(run.spec.attempt_id) {
            return Ok(());
        }
        let attempt = engine
            .control_db()
            .attempt(run.spec.task_id, run.spec.attempt_id)
            .map_err(|_| "attempt不可用")?
            .ok_or("attempt缺失")?;
        if attempt.fence != run.spec.fence {
            return Err("attempt围栏不匹配".into());
        }
        // T1 reservations can only roll forward. Keep that claim fenced while the engine recovers.
        if matches!(
            attempt.phase,
            AttemptPhase::Committing | AttemptPhase::Completed
        ) {
            return Ok(());
        }
        if run.launch_failed
            && attempt.phase == AttemptPhase::Running
            && attempt.lease_until_ms > now
        {
            if !isolated {
                return Ok(());
            }
            // The engine owns durable audit, retry_on, backoff and the three-attempt ceiling.
            // It independently refuses any attempt with persisted tool calls.
            return engine
                .launch_failed(run.spec.task_id, run.spec.attempt_id, run.spec.fence, now)
                .map(|_| ())
                .map_err(|_| "启动失败收尾待恢复".into());
        }
        if attempt.phase == AttemptPhase::Running {
            engine
                .command(run.spec.task_id, TaskCommand::Cancel, now)
                .map_err(|_| "取消封闭待恢复")?;
        }
        // Isolation without a valid CLI receipt is not a stopped receipt. Preserve the
        // original 180-second window; periodic reconciliation records abandoned on expiry.
        Ok(())
    }

    /// Cancels one owned task before signalling the isolated process.
    pub fn cancel(&self, task: studiolo_core::TaskId) -> Result<(), String> {
        let now = jiff::Timestamp::now().as_millisecond();
        self.authority
            .board
            .engine
            .lock()
            .map_err(|_| "黑板不可用")?
            .command(task, TaskCommand::Cancel, now)
            .map_err(|_| "取消命令失败")?;
        self.tick()
    }
}
fn unit(run: &ActiveRun) -> String {
    format!(
        "studiolo-runner-{}-{}.scope",
        run.spec.attempt_id,
        run.spec.fence.get()
    )
}

fn launch_failure(
    error: &studiolo_runner_adapter::RunnerError,
    stopping: bool,
    observed_tools: Option<u64>,
    persisted_tools: Option<u64>,
) -> bool {
    matches!(
        error,
        studiolo_runner_adapter::RunnerError::LaunchFailed { .. }
    ) && !stopping
        && observed_tools == Some(0)
        && persisted_tools == Some(0)
}
#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
