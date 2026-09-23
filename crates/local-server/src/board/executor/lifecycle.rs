//! 按尝试逐个维护：一个候选失败绝不打断其他受监管的运行。
use super::{ActiveRun, RunnerPlane};
use std::sync::atomic::Ordering;
use studiolo_control_db::AttemptPhase;
use studiolo_runner_adapter::{RunnerAdapter, RunnerObservation};
use studiolo_task_engine::{TaskCommand, TaskEngine};

impl RunnerPlane {
    /// 按尝试各自独立地复核租约／`fence`／撤销状态，并暂存候选。
    /// 须在 `spawn_blocking` 中、引擎锁之外调用，每个维护周期至多一次。
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
                // 已保留的补丁与引擎的任何预留仍可供恢复使用。
                // 这里的任何错误都不会重启 `CLI`，也不妨碍下一个尝试接受监管。
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
                // 心跳绝不为已取消或已撤销的尝试续期。
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
                // `RealRunner` 返回这份报告之前，已把候选保存到 `scratch` 之外。
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
                // 预算触发的取消可能与进程退出同时被首次观察到。
                let _ = engine.command(run.spec.task_id, TaskCommand::Cancel, now);
                run.summary.state = "candidate_retained".into();
                if let Ok(receipt) = serde_json::from_value(receipt)
                    && engine
                        .stage_board_stopped_candidates(report, receipt, now)
                        .is_ok()
                {
                    run.summary.state = "stopped".into();
                }
                // 无效或迟到的停止包仍予保留；只有 T1 恢复或原定的取消期限
                // 才能关闭该认领，绝不凭编造的回执关闭。
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

    /// 先移除生效中的授权，再删除它的短期票据。即使删除失败，尝试也保持
    /// 已撤权；维护重试绝不把残留文件当作授权依据。
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
        // 即使某一步失败，也执行完全部相互独立的清理步骤。
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
        // T1 预留只能前滚；引擎恢复期间让该认领继续受 `fence` 约束。
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
            // 持久审计、`retry_on`、退避与三次尝试上限都归引擎负责。
            // 引擎也会独立拒绝任何已有持久化工具调用的尝试。
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
        // 只有隔离而没有有效的 `CLI` 回执，不算停止回执。保留原定的
        // 180 秒窗口；到期后由周期对账记为 `abandoned`。
        Ok(())
    }

    /// 先取消一个自有任务，再向已隔离的进程发信号。
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
