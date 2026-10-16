//! 先挂接重启后仍可识别的身份，之后任何能力才能到达活进程。
use super::{ActiveRun, Binding, RunSummary, RunnerPlane, StartRequest};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::atomic::Ordering,
};
use studiolo_audit_ledger::AuditPhase;
use studiolo_runner_adapter::{RealRunner, RunnerAdapter, RunnerRef};
use studiolo_task_engine::TaskEngine;

impl RunnerPlane {
    /// 启动一个显式选定、经认证的工作项；主人批准须由 HTTP 层强制执行。
    pub fn start_task(&self, r: StartRequest) -> Result<RunSummary, String> {
        if self.authority.frozen.load(Ordering::SeqCst) {
            return Err("Runner全局冻结".into());
        }
        let _admission = self.admission.start(&self.authority.frozen)?;
        let mut runs = self.runs.lock().map_err(|_| "Runner状态不可用")?;
        let request = self.authorization_request(&r)?;
        let auth = self
            .authority
            .board
            .authorize(&request)
            .map_err(String::from)?;
        if let Some(existing) = runs.values().find(|run| {
            run.summary.task_id == r.task_id
                && (run.summary.state != "failed" || run.cleanup_pending)
        }) {
            return Ok(existing.summary.clone());
        }
        if runs
            .values()
            .any(|run| run.summary.state == "running" || run.cleanup_pending)
        {
            return Err("Runner v1仅允许一个活跃执行体".into());
        }
        if self.authority.frozen.load(Ordering::SeqCst) {
            return Err("Runner全局冻结".into());
        }
        let mapped = self
            .authority
            .config
            .projects
            .iter()
            .find(|p| p.project == request.project)
            .ok_or("项目没有受信仓库映射")?;
        let _source_lease = super::landing_store::Lease::acquire(
            &self.authority.config.state_root,
            &mapped.repository,
        )?;
        let now = jiff::Timestamp::now().as_millisecond();
        let id = format!("runner-start:{}:{}", r.task_id, ulid::Ulid::generate());
        self.authority
            .board
            .event(&request, &id, AuditPhase::Intent, "admitted")?;
        let claim = {
            let mut engine = self
                .authority
                .board
                .engine
                .lock()
                .map_err(|_| "黑板不可用")?;
            engine
                .claim_board_task(r.task_id, &r.instance_id, now.saturating_add(120_000), now)
                .map_err(|_| "显式claim失败")?
        };
        let prepared = self.prepare(&claim, &r, &auth.hash);
        let (spec, grant, profile, project) = match prepared {
            Ok(v) => v,
            Err(reason) => {
                {
                    let mut engine = self
                        .authority
                        .board
                        .engine
                        .lock()
                        .map_err(|_| "黑板不可用")?;
                    let _ =
                        engine.command(r.task_id, studiolo_task_engine::TaskCommand::Cancel, now);
                    let _ = engine.acknowledge_cancel(
                        r.task_id,
                        claim.attempt.attempt_id,
                        claim.attempt.fence,
                        now,
                    );
                }
                self.authority.board.event(
                    &request,
                    &id,
                    AuditPhase::Outcome,
                    "preparation_denied",
                )?;
                return Err(reason);
            }
        };
        let unit = format!(
            "studiolo-runner-{}-{}.scope",
            spec.attempt_id,
            spec.fence.get()
        );
        let mut retryable_launch = false;
        let outcome = (|| {
            self.authority
                .board
                .engine
                .lock()
                .map_err(|_| "黑板不可用")?
                .attach_runner(
                    spec.task_id,
                    spec.attempt_id,
                    spec.fence,
                    RunnerRef::opaque(profile.provider.name(), &unit),
                    now,
                )
                .map_err(|_| "执行体身份持久化失败")?;
            self.authority
                .bindings
                .lock()
                .map_err(|_| "Runner授权不可用")?
                .insert(
                    spec.attempt_id,
                    Binding {
                        agent_type: r.agent_type.clone(),
                        instance_id: r.instance_id.clone(),
                        project: project.project.clone(),
                        grant: Some(grant.clone()),
                    },
                );
            let ticket = self
                .relay
                .issue(grant)
                .map_err(|_| "模型外发或预算准入拒绝")?;
            let ticket_path = self
                .authority
                .config
                .state_root
                .join(format!("{}.capability", spec.attempt_id));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&ticket_path)
                .map_err(|_| "短期capability文件创建失败")?;
            file.write_all(ticket.expose().as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|_| "短期capability持久化失败")?;
            File::open(&self.authority.config.state_root)
                .and_then(|f| f.sync_all())
                .map_err(|_| "短期capability目录持久化失败")?;
            let runner =
                RealRunner::new(self.runner_config(&profile, &project, ticket_path.clone()));
            let started = runner.start(&spec).map_err(|error| {
                retryable_launch = matches!(
                    error,
                    studiolo_runner_adapter::RunnerError::LaunchFailed { .. }
                );
                "隔离进程启动失败或准入条件变化；未重试网络"
            })?;
            if started.runner_ref != RunnerRef::opaque(profile.provider.name(), &unit) {
                return Err("执行体引用不匹配");
            }
            let summary = RunSummary {
                task_id: spec.task_id,
                attempt_id: spec.attempt_id,
                adapter: profile.provider.name().into(),
                state: "running".into(),
                progress: Default::default(),
            };
            runs.insert(
                spec.attempt_id,
                ActiveRun {
                    runner,
                    handle: started.handle,
                    spec: spec.clone(),
                    summary: summary.clone(),
                    ticket: ticket_path,
                    cleanup_pending: false,
                    launch_failed: false,
                },
            );
            Ok::<_, &str>(summary)
        })();
        if outcome.is_err() {
            let ticket_path = self
                .authority
                .config
                .state_root
                .join(format!("{}.capability", spec.attempt_id));
            let _ = self.revoke_ticket(spec.attempt_id, &ticket_path);
            let isolated = super::recovery::isolate_unit(&unit);
            if let Ok(mut engine) = self.authority.board.engine.lock() {
                let now = jiff::Timestamp::now().as_millisecond();
                if isolated.is_ok() && retryable_launch {
                    // 只有带类型、发生在任何工具调用之前的启动失败，才能进入引擎重试策略。
                    let _ = engine.launch_failed(spec.task_id, spec.attempt_id, spec.fence, now);
                } else {
                    let _ = engine.command(
                        spec.task_id,
                        studiolo_task_engine::TaskCommand::Cancel,
                        now,
                    );
                    if isolated.is_ok() {
                        let _ = engine.acknowledge_cancel(
                            spec.task_id,
                            spec.attempt_id,
                            spec.fence,
                            now,
                        );
                    } else {
                        self.admission.invalidate();
                        self.authority.frozen.store(true, Ordering::SeqCst);
                        let _ = self.relay.set_frozen(true);
                    }
                }
            }
            // 即使隔离或票据清理失败，下面仍会尝试写审计。
        }
        self.authority.board.event(
            &request,
            &id,
            AuditPhase::Outcome,
            if outcome.is_ok() {
                "started"
            } else if retryable_launch {
                "launch_failed"
            } else {
                "start_denied_or_incomplete"
            },
        )?;
        outcome.map_err(str::to_owned)
    }
}
