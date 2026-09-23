//! 跨 Control DB、VaultBroker 与 AuditLedger 的持久化任务生命周期编排。
//!
//! 本模块持有唯一可变的 [`ControlDb`] 句柄。Vault 读取与快照
//! 都在认领事务之前完成；终态产物发布使用持久的
//! T1 预留与「manifest 最后写入」的前滚。

#![forbid(unsafe_code)]

mod board;
mod board_audit;
mod board_projection;
mod board_types;
mod lifecycle;
mod numbering;
mod projection;
mod terminal_staging;
pub use board_types::*;

use serde::{Deserialize, Serialize};
use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_control_db::{
    AttemptPhase, AttemptRecord, ControlDb, DbError, OutboxRecord, PendingTerminalAudit,
    SnapshotEntryRecord, SnapshotRecord, TaskRecord, TaskStatus, TerminalReservation,
};
use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_runner_adapter::{RunInput, RunSpec, RunnerRef, RunnerReport};
use studiolo_vault_broker::{
    ArtifactCandidate, ArtifactCommit, ArtifactCommitReceipt, ArtifactStager, BrokerError,
    Mutation, Precondition, Snapshot, SnapshotId, SnapshotInput, VaultBroker,
};

/// 一条投影事件在单轮里最多尝试的写入次数（契约 §6「每条事件最多尝试三次」）。
const MAX_PROJECTION_ATTEMPTS: usize = 3;

/// 人写的重试声明。M0 Stub 任务只记录不重试；黑板启动失败的重试规则见各字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    /// 允许的 attempt 总数上限（含首次）；启动失败重试同样受此约束。
    pub max_attempts: u32,
    /// 重试前的最短等待秒数；启动失败重试以此作为退避。
    pub backoff_seconds: u64,
    /// 显式列出可重试的结果名，空则不自动重试；目前只认 `launch_failure`（见 `launch_failed`）。
    pub retry_on: Vec<String>,
}

/// 人写的任务定义，刻意不含 TaskEngine 投影字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    /// 作者选定的逻辑角色。
    pub role: String,
    /// runner 适配器类型。
    pub adapter: String,
    /// 需要快照的显式规范输入。
    pub input_paths: Vec<VaultPath>,
    /// 任务允许的 Web 目标，尚未与策略取交集。
    pub allow_web: Vec<String>,
    /// 任务允许的数据库操作，尚未与策略取交集。
    pub allow_db: Vec<String>,
    /// 记录在案的重试声明；M0 不自动执行，只有 `launch_failed` 按它处理启动失败重试。
    pub retry_policy: RetryPolicy,
    /// attempt 超时秒数。
    pub timeout: u64,
    /// 人类可读的项目名。
    pub project: String,
    /// 可选的父任务。
    pub parent: Option<TaskId>,
    /// 可选的展示标题。
    pub title: Option<String>,
    /// 可选的 runner 指令。
    pub instruction: Option<String>,
}

impl TaskSpec {
    fn validate(&self) -> Result<(), TaskEngineError> {
        if self.adapter != "stub" {
            return Err(TaskEngineError::InvalidTaskSpec(
                "M0 only enables the in-process stub adapter",
            ));
        }
        self.validate_board()
    }

    fn validate_board(&self) -> Result<(), TaskEngineError> {
        if self.role.trim().is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec("role is empty"));
        }
        if !matches!(
            self.adapter.as_str(),
            "stub" | "consumer" | "claude" | "codex"
        ) {
            return Err(TaskEngineError::InvalidTaskSpec(
                "unsupported board adapter",
            ));
        }
        if self.project.trim().is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec("project is empty"));
        }
        if self.retry_policy.max_attempts == 0 {
            return Err(TaskEngineError::InvalidTaskSpec(
                "retry_policy.max_attempts is zero",
            ));
        }
        if self.timeout == 0 {
            return Err(TaskEngineError::InvalidTaskSpec("timeout is zero"));
        }
        let mut paths = self.input_paths.clone();
        paths.sort();
        if paths.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(TaskEngineError::InvalidTaskSpec(
                "input_paths contains duplicates",
            ));
        }
        Ok(())
    }
}

/// M0 支持的封闭任务命令集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskCommand {
    /// 把 draft 移入认领队列。
    Run,
    /// 撤销 draft、queued 或 running 的工作。
    Cancel,
}

/// 一次认领尝试的确定性输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRequest {
    /// 由组合根提供的已认证 worker 身份。
    pub worker_id: String,
    /// 当前 Unix 时间（毫秒）。
    pub now_ms: i64,
    /// 租约截止时间（Unix 毫秒）。
    pub lease_until_ms: i64,
}

/// 认领事务提交后返回的不可变运行请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedTask {
    /// 随 attempt 持久化的快照标识。
    pub snapshot_id: String,
    /// 面向 runner 的不可变请求。
    pub run_spec: RunSpec,
}

/// 任务定义与 Control DB 投影分字段存放的视图。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    /// 稳定的任务标识。
    pub task_id: TaskId,
    /// 只含人写定义。
    pub spec: TaskSpec,
    /// 权威运行时状态。
    pub status: TaskStatus,
    /// 当前 attempt（若有）。
    pub active_attempt: Option<AttemptId>,
    /// 最近一次结束的 attempt（若有）。
    pub last_attempt: Option<AttemptId>,
    /// 不可变输入快照：Run 确认（黑板任务在认领前冻结）时写入，认领事务只沿用、不另选。
    pub input_snapshot: Option<String>,
}

/// 终态报告处理成功的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportResult {
    /// 处理后的任务状态（现恒为 NeedsReview，等待审阅）。
    pub status: TaskStatus,
    /// 「manifest 最后写入」的 Broker 回执。
    pub receipt: ArtifactCommitReceipt,
    /// 同一持久预留是否早已进入终态（即本次为重放）。
    pub replayed: bool,
}

/// 一次重启对账完成的工作。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// 最先被撤销并放弃的运行中 attempt 数。
    pub abandoned_attempts: usize,
    /// 重启隔离后完成的、先前已请求的取消数。
    pub cancelled_attempts: usize,
    /// 前滚到待审阅的已持久 committing 预留数。
    pub committed_attempts: usize,
    /// 从持久审计 outbox 补投的、先前已落定的终态结果数。
    pub audit_outcomes: usize,
}

/// TaskEngine 接缝对外暴露的失败。
#[derive(Debug, thiserror::Error)]
pub enum TaskEngineError {
    /// 人写定义违反公开 schema，或黑板相关请求未通过校验。
    #[error("invalid task specification: {0}")]
    InvalidTaskSpec(&'static str),
    /// 任务或 attempt 不存在。
    #[error("task engine record not found: {0}")]
    NotFound(&'static str),
    /// 持久状态不允许请求的命令。
    #[error("invalid task transition from {from} via {operation}")]
    InvalidTransition {
        /// Control DB 观察到的持久状态（入参不合法时为原因说明）。
        from: String,
        /// 稳定的操作名。
        operation: &'static str,
    },
    /// attempt 身份或 fence 已不再权威。
    #[error("stale attempt fence")]
    StaleFence,
    /// 另一个终态迁移已经胜出。
    #[error("attempt is already terminal")]
    AlreadyTerminal,
    /// 同一事件或请求标识被以不同内容重放。
    #[error("idempotency identity was reused with different content")]
    IdempotencyMismatch,
    /// 权威审计接收端不可用；受保护的副作用已停止。
    #[error("audit ledger is unavailable")]
    AuditUnavailable,
    /// 审计校验或持久化失败。
    #[error("audit operation failed: {0}")]
    Audit(#[source] AuditError),
    /// Vault 读取或快照等 Broker 操作失败。
    #[error("Vault Broker operation failed: {0}")]
    Broker(#[source] BrokerError),
    /// 「manifest 最后写入」的产物提交失败；预留仍在，留待对账前滚。
    #[error("artifact commit failed: {0}")]
    CommitFailed(#[source] BrokerError),
    /// Control DB 在已归类迁移之外失败（含黑板规则拒绝等未单独映射的错误）。
    #[error("Control DB operation failed: {0}")]
    ControlDb(#[source] DbError),
    /// JSON 序列化或解析失败。
    #[error("task engine JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
    /// 另一个活动认领占有了请求的路径。
    #[error("paths held by {by_instance}: {paths:?}")]
    PathsHeld {
        #[doc = "持有冲突认领的实例。"]
        by_instance: String,
        #[doc = "与活动认领相交的请求路径。"]
        paths: Vec<String>,
    },
    /// 完整性校验失败：持久记录、快照或产物指针与预期不符（如终态预留与其 attempt 不匹配）。
    #[error("task engine integrity failure: {0}")]
    Integrity(String),
    /// 投影失败，例如任务页无法在不丢失作者内容的前提下更新。
    #[error("task projection failed: {0}")]
    Projection(String),
    /// 部分事件因各自任务页拒绝而仍待投递；同一轮中其他
    /// 任务的事件照常投递。
    #[error("task projection incomplete: {delivered} delivered, {} task(s) left pending", failures.len())]
    ProjectionIncomplete {
        /// 本轮投递的事件数。
        delivered: usize,
        /// 每个被隔离任务的首个被拒事件；该任务后续事件本轮跳过。
        failures: Vec<ProjectionFailure>,
    },
    /// 任务等待认领期间，已确认的输入发生了变化。
    #[error("task {0} requires input reconfirmation")]
    NeedsReconfirmation(TaskId),
    /// runner 引用无效：适配器与任务不符、缺少稳定标识，或带有 M0 不接受的 PID/启动时刻。
    #[error("invalid or mismatched runner reference")]
    InvalidRunnerRef,
}

/// 因所属任务页拒绝而留待投递的一条投影事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionFailure {
    /// 拒绝该投影的任务页所属的任务。
    pub task_id: TaskId,
    /// 仍待投递的 outbox 序号。
    pub sequence: i64,
    /// 稳定、人类可读的拒绝原因。
    pub reason: String,
}

/// 同步、对象安全的任务生命周期接缝。
pub trait TaskEngine: Send {
    /// 持久化一份已校验的人写 draft。
    fn create_task(&mut self, spec: TaskSpec, now_ms: i64) -> Result<TaskId, TaskEngineError>;

    /// 执行一条封闭任务命令，并返回执行后的视图。
    fn command(
        &mut self,
        task_id: TaskId,
        command: TaskCommand,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError>;

    /// 加载 Run 确认的快照，原子地认领最早入队的 queued 任务。
    fn claim(&mut self, request: ClaimRequest) -> Result<Option<ClaimedTask>, TaskEngineError>;

    /// 在 attempt fence 下持久化 runner 的重启安全身份。
    fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref: RunnerRef,
        now_ms: i64,
    ) -> Result<(), TaskEngineError>;

    /// 在 attempt fence 下续期有效租约。
    fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), TaskEngineError>;

    /// 组合根停止 runner 后完成取消。
    fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError>;

    /// 经 VaultBroker 以 CAS 重放待投递的受保护字段投影。
    fn project_pending(&mut self) -> Result<usize, TaskEngineError>;

    /// 预留结构化 runner 报告，并经审计与 Broker 提交将其前滚。
    fn report(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError>;

    /// 撤销遗留的运行中工作，再前滚持久化的 committing 预留。
    fn reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, TaskEngineError>;

    /// 返回单个任务，定义与投影分开存放。
    fn task(&self, task_id: TaskId) -> Result<Option<TaskView>, TaskEngineError>;
}

/// 持久化的 M0 适配器；独占持有 Control DB 与 Broker 写入端。
pub struct PersistentTaskEngine<B, L> {
    control_db: ControlDb,
    broker: B,
    audit_ledger: L,
}

impl<B, L> PersistentTaskEngine<B, L> {
    /// 接管 Control DB 与 Broker 写入端的独占所有权。
    pub fn new(control_db: ControlDb, broker: B, audit_ledger: L) -> Self {
        Self {
            control_db,
            broker,
            audit_ledger,
        }
    }

    /// 只读访问，供组合与重启断言使用。
    #[must_use]
    pub fn control_db(&self) -> &ControlDb {
        &self.control_db
    }

    /// 只读访问所持有的 Broker 适配器。
    #[must_use]
    pub fn broker(&self) -> &B {
        &self.broker
    }

    /// 只读访问所持有的审计适配器。
    #[must_use]
    pub fn audit_ledger(&self) -> &L {
        &self.audit_ledger
    }

    /// 交出所持有的各部件，供组合根显式移交。
    #[must_use]
    pub fn into_parts(self) -> (ControlDb, B, L) {
        (self.control_db, self.broker, self.audit_ledger)
    }
}

impl<B, L> TaskEngine for PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    fn create_task(&mut self, spec: TaskSpec, now_ms: i64) -> Result<TaskId, TaskEngineError> {
        spec.validate()?;
        let task_id = TaskId::new();
        let definition = serde_json::to_value(spec)?;
        self.control_db
            .create_task(task_id, &definition, now_ms)
            .map_err(map_db)?;
        Ok(task_id)
    }

    fn command(
        &mut self,
        task_id: TaskId,
        command: TaskCommand,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        let current = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        match (command, current.status) {
            (TaskCommand::Run, TaskStatus::Draft) => {
                if self
                    .control_db
                    .board_task(task_id)
                    .map_err(map_db)?
                    .is_some()
                {
                    return Err(TaskEngineError::InvalidTaskSpec(
                        "board tasks queue through claim_board_task",
                    ));
                }
                // Run 只要求本任务自己的投影已交付；别的任务的坏页不能挡住这条命令。
                self.project_task(task_id)?;
                let snapshot = self.capture_inputs(task_id, &current.spec)?;
                self.control_db
                    .queue_with_snapshot(task_id, &snapshot_record(&snapshot), now_ms)
                    .map_err(map_db)?;
            }
            (TaskCommand::Run, TaskStatus::Queued)
            | (TaskCommand::Cancel, TaskStatus::Cancelled | TaskStatus::CancelRequested) => {}
            (TaskCommand::Cancel, TaskStatus::Draft | TaskStatus::Queued | TaskStatus::Running) => {
                self.control_db
                    .cancel_task(task_id, now_ms)
                    .map_err(map_db)?;
            }
            (
                TaskCommand::Cancel,
                TaskStatus::Committing
                | TaskStatus::NeedsReview
                | TaskStatus::Approved
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Abandoned,
            ) => {
                return Err(TaskEngineError::AlreadyTerminal);
            }
            (TaskCommand::Run, status) | (TaskCommand::Cancel, status) => {
                return Err(TaskEngineError::InvalidTransition {
                    from: format!("{status:?}"),
                    operation: match command {
                        TaskCommand::Run => "run",
                        TaskCommand::Cancel => "cancel",
                    },
                });
            }
        }
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn claim(&mut self, request: ClaimRequest) -> Result<Option<ClaimedTask>, TaskEngineError> {
        if request.worker_id.trim().is_empty() || request.lease_until_ms <= request.now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "invalid claim input".to_owned(),
                operation: "claim",
            });
        }
        let Some(task) = self.control_db.next_queued().map_err(map_db)? else {
            return Ok(None);
        };
        let spec = decode_spec(&task)?;

        // 所有 Vault I/O 都在认领事务之前完成；队列从不选取新的输入字节。
        let snapshot_id: SnapshotId =
            serde_json::from_value(json!(task.input_snapshot.as_ref().ok_or_else(|| {
                TaskEngineError::Integrity("queued task has no confirmed snapshot".to_owned())
            })?))?;
        let snapshot = self
            .broker
            .load_snapshot(&snapshot_id)
            .map_err(TaskEngineError::Broker)?;
        let snapshot_record = snapshot_record(&snapshot);
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)?
            || self
                .control_db
                .snapshot(snapshot.id.as_str())
                .map_err(map_db)?
                .as_ref()
                != Some(&snapshot_record)
        {
            return Err(TaskEngineError::Integrity(
                "confirmed snapshot does not match Control DB metadata or Vault identity"
                    .to_owned(),
            ));
        }
        if !self.inputs_unchanged(&task, &snapshot)? {
            self.control_db
                .require_reconfirmation(task.task_id, request.now_ms)
                .map_err(map_db)?;
            return Err(TaskEngineError::NeedsReconfirmation(task.task_id));
        }
        let attempt = self
            .control_db
            .claim(
                task.task_id,
                &request.worker_id,
                snapshot_record,
                request.now_ms,
                request.lease_until_ms,
            )
            .map_err(map_db)?;
        let run_spec = RunSpec {
            task_id: task.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            budget: Default::default(),
            execution: None,
            instruction: spec
                .instruction
                .clone()
                .or(spec.title.clone())
                .unwrap_or_default(),
            inputs: snapshot
                .files
                .into_iter()
                .map(|file| RunInput {
                    path: file.path,
                    revision: file.revision.to_string(),
                    content: file.content,
                })
                .collect(),
        };
        Ok(Some(ClaimedTask {
            snapshot_id: attempt.snapshot_id,
            run_spec,
        }))
    }

    fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref: RunnerRef,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        let task = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        if runner_ref.adapter != task.spec.adapter
            || runner_ref.opaque_id.trim().is_empty()
            || runner_ref.pid.is_some()
            || runner_ref.start_time_ticks.is_some()
        {
            return Err(TaskEngineError::InvalidRunnerRef);
        }
        let encoded = serde_json::to_value(runner_ref)?;
        self.control_db
            .attach_runner(task_id, attempt_id, fence, &encoded, now_ms)
            .map_err(map_db)
    }

    fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        if lease_until_ms <= now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "expired lease".to_owned(),
                operation: "heartbeat",
            });
        }
        self.control_db
            .heartbeat(task_id, attempt_id, fence, lease_until_ms, now_ms)
            .map_err(map_db)
    }

    fn report(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError> {
        self.verify_vault_binding()?;
        let event_id = AuditEventId::parse(event_id.to_owned()).map_err(map_audit)?;
        self.migrate_terminal_bodies()?;
        let prior = self
            .control_db
            .attempt(report.task_id, report.attempt_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("attempt"))?;
        if prior.fence != report.fence {
            return Err(TaskEngineError::StaleFence);
        }
        if self
            .control_db
            .board_task(report.task_id)
            .map_err(map_db)?
            .is_some()
        {
            return Err(TaskEngineError::InvalidTaskSpec(
                "board tasks require structured board receipt",
            ));
        }
        if prior.phase == AttemptPhase::Running
            && !self
                .control_db
                .is_attempt_active_at(report.task_id, report.attempt_id, report.fence, now_ms)
                .map_err(map_db)?
        {
            return Err(TaskEngineError::StaleFence);
        }
        if !matches!(
            prior.phase,
            AttemptPhase::Running | AttemptPhase::Committing | AttemptPhase::Completed
        ) {
            return Err(TaskEngineError::AlreadyTerminal);
        }
        let reservation = self.stage_reservation(&event_id, &report)?;
        let replayed = match prior.phase {
            AttemptPhase::Running => {
                self.control_db
                    .reserve_terminal(
                        report.task_id,
                        report.attempt_id,
                        report.fence,
                        &reservation,
                        now_ms,
                    )
                    .map_err(map_db)?;
                false
            }
            AttemptPhase::Committing | AttemptPhase::Completed => {
                if prior.terminal.as_ref() != Some(&reservation) {
                    return Err(TaskEngineError::IdempotencyMismatch);
                }
                prior.phase == AttemptPhase::Completed
            }
            AttemptPhase::Claimed
            | AttemptPhase::CancelRequested
            | AttemptPhase::Failed
            | AttemptPhase::Cancelled
            | AttemptPhase::Abandoned => return Err(TaskEngineError::AlreadyTerminal),
        };
        let receipt = if replayed {
            let receipt = self
                .control_db
                .artifact_receipt(prior.task_id, prior.attempt_id)
                .map_err(map_db)?
                .ok_or_else(|| {
                    TaskEngineError::Integrity("completed attempt has no receipt".to_owned())
                })?;
            self.deliver_pending_outcomes(Some(prior.attempt_id), now_ms)?;
            serde_json::from_value(receipt)?
        } else {
            self.roll_forward(&prior_identity(&prior), &reservation, now_ms)?
        };
        Ok(ReportResult {
            status: TaskStatus::NeedsReview,
            receipt,
            replayed,
        })
    }

    fn reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, TaskEngineError> {
        self.verify_vault_binding()?;
        if !self.control_db.board_tasks().map_err(map_db)?.is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec(
                "board recovery requires explicit reconcile_startup or reconcile_periodic",
            ));
        }
        self.migrate_terminal_bodies()?;
        let abandoned_attempts = self.control_db.abandon_inflight(now_ms).map_err(map_db)?;
        // M0 只运行进程内 Stub；进程重启本身已隔离了它的工作。
        let cancelling = self.control_db.cancelling_attempts().map_err(map_db)?;
        for attempt in &cancelling {
            self.control_db
                .acknowledge_cancel(attempt.task_id, attempt.attempt_id, attempt.fence, now_ms)
                .map_err(map_db)?;
        }
        let audit_outcomes = self.deliver_pending_outcomes(None, now_ms)?;
        let committing = self.control_db.committing_attempts().map_err(map_db)?;
        let mut committed_attempts = 0;
        for attempt in committing {
            let reservation = attempt.terminal.clone().ok_or_else(|| {
                TaskEngineError::Integrity(format!(
                    "committing attempt {} has no reservation",
                    attempt.attempt_id
                ))
            })?;
            self.roll_forward(&prior_identity(&attempt), &reservation, now_ms)?;
            committed_attempts += 1;
        }
        Ok(ReconcileReport {
            abandoned_attempts,
            cancelled_attempts: cancelling.len(),
            committed_attempts,
            audit_outcomes,
        })
    }

    fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        self.control_db
            .acknowledge_cancel(task_id, attempt_id, fence, now_ms)
            .map_err(map_db)?;
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn project_pending(&mut self) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self.control_db.pending_outbox().map_err(map_db)?;
        self.project_events(pending)
    }

    fn task(&self, task_id: TaskId) -> Result<Option<TaskView>, TaskEngineError> {
        self.control_db
            .task(task_id)
            .map_err(map_db)?
            .map(|record| task_view(&record))
            .transpose()
    }
}

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    /// 只投递单个任务的待投递事件，按序号顺序。
    fn project_task(&mut self, task_id: TaskId) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self
            .control_db
            .pending_outbox()
            .map_err(map_db)?
            .into_iter()
            .filter(|event| event.task_id == task_id)
            .collect();
        self.project_events(pending)
    }

    /// 按序号顺序投递事件。页面级拒绝会在本轮剩余部分隔离该任务
    /// （其后续事件保持待投递，维持单任务内的顺序），但不阻塞其他
    /// 任务；存储与 Broker 故障仍会中止整轮。
    fn project_events(&mut self, events: Vec<OutboxRecord>) -> Result<usize, TaskEngineError> {
        let mut delivered = 0;
        let mut failures: Vec<ProjectionFailure> = Vec::new();
        for event in events {
            if failures
                .iter()
                .any(|failure| failure.task_id == event.task_id)
            {
                continue;
            }
            match self.project_event(&event) {
                Ok(()) => delivered += 1,
                Err(error) if is_page_rejection(&error) => failures.push(ProjectionFailure {
                    task_id: event.task_id,
                    sequence: event.sequence,
                    reason: error.to_string(),
                }),
                Err(error) => return Err(error),
            }
        }
        if failures.is_empty() {
            Ok(delivered)
        } else {
            Err(TaskEngineError::ProjectionIncomplete {
                delivered,
                failures,
            })
        }
    }

    fn project_event(&mut self, event: &OutboxRecord) -> Result<(), TaskEngineError> {
        if self
            .control_db
            .board_task(event.task_id)
            .map_err(map_db)?
            .is_some()
        {
            return Err(TaskEngineError::Projection(
                "board task events require project_board batch publication".into(),
            ));
        }
        {
            let page = task_page_path(event.task_id)?;
            let vault_uuid = self.broker.vault_uuid();
            let mut succeeded = false;
            // 幂等键的判别位：只在「前提当下仍然成立、拒绝只可能是旧键终态的重放」时推进，
            // 详见 `projection::key`。
            let mut generation: u32 = 0;
            for _ in 0..MAX_PROJECTION_ATTEMPTS {
                let mutation = match self.broker.read(&page) {
                    Ok(current) => {
                        projection::validate_task_page(&current.content, event.task_id)
                            .map_err(TaskEngineError::Projection)?;
                        let content =
                            projection::apply_projection(&current.content, &event.payload_json)
                                .map_err(TaskEngineError::Projection)?;
                        if content == current.content {
                            succeeded = true;
                            break;
                        }
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::Match(current.revision),
                            &content,
                            generation,
                        );
                        Mutation::replace(page.clone(), content, current.revision, key)
                    }
                    Err(BrokerError::NotFound(_))
                        if projected_status(&event.payload_json) == Some(TaskStatus::Draft) =>
                    {
                        let task = self
                            .control_db
                            .task(event.task_id)
                            .map_err(map_db)?
                            .ok_or(TaskEngineError::NotFound("projection task"))?;
                        let created = jiff::Timestamp::from_millisecond(task.created_at_ms)
                            .map_err(|error| TaskEngineError::Projection(error.to_string()))?
                            .to_string();
                        let content = projection::render_task_page(
                            event.task_id,
                            &decode_spec(&task)?,
                            &created,
                        )
                        .map_err(TaskEngineError::Projection)?;
                        let content = projection::apply_projection(&content, &event.payload_json)
                            .map_err(TaskEngineError::Projection)?;
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::MustNotExist,
                            &content,
                            generation,
                        );
                        Mutation::create(page.clone(), content, key)
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                };
                let precondition = mutation.precondition;
                let expected_content = mutation.content.clone();
                match self.broker.mutate(mutation) {
                    Ok(receipt) => {
                        // 幂等回执只证明历史上写入过，并不证明外部编辑者
                        // 此后一直没有改动那些已安装的字节。
                        let installed = self.broker.read(&page).map_err(TaskEngineError::Broker)?;
                        if installed.revision == receipt.revision
                            && installed.content == expected_content
                        {
                            succeeded = true;
                            break;
                        }
                    }
                    Err(BrokerError::RevisionConflict { .. }) => {
                        // 拒绝要么是对当前状态的判定，要么是 Broker 为这把键封存的拒绝终态的
                        // 重放。前提当下仍然成立 ⇒ 只可能是重放：推进判别位换一把没被封死的
                        // 键，否则这条事件会永远撞在同一个终态上（T-028 起 CAS 拒绝按键终态）。
                        // 前提确实已变时不推进：新前提本身就会派生出新键。
                        if self.premise_still_holds(&page, precondition)? {
                            generation = generation.saturating_add(1);
                        }
                        continue;
                    }
                    Err(BrokerError::IdempotencyConflict) => {
                        // 同一把键被另一个请求占用：明确拒绝，绝不静默继承它的终态。
                        // 只隔离本任务，别的任务的投影照常交付。
                        return Err(TaskEngineError::Projection(format!(
                            "task {} projection idempotency key is already bound to a different request",
                            event.task_id
                        )));
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                }
            }
            if !succeeded {
                return Err(TaskEngineError::Projection(format!(
                    "task {} changed during {MAX_PROJECTION_ATTEMPTS} projection attempts",
                    event.task_id
                )));
            }
            self.control_db
                .mark_outbox_delivered(event.sequence, jiff::Timestamp::now().as_millisecond())
                .map_err(map_db)?;
        }
        Ok(())
    }

    /// 本次写入的前提（不存在 / 指定 revision）当下是否仍然成立。
    ///
    /// 只用来解释一次 CAS 拒绝：前提仍成立时，这次拒绝不可能是对当前状态的判定，
    /// 只能是 Broker 重放同一把幂等键上的旧拒绝终态。
    fn premise_still_holds(
        &self,
        page: &VaultPath,
        precondition: Precondition,
    ) -> Result<bool, TaskEngineError> {
        match (self.broker.read(page), precondition) {
            (Ok(current), Precondition::Match(expected)) => Ok(current.revision == expected),
            (Ok(_), Precondition::MustNotExist) => Ok(false),
            (Err(BrokerError::NotFound(_)), Precondition::MustNotExist) => Ok(true),
            (Err(BrokerError::NotFound(_)), Precondition::Match(_)) => Ok(false),
            (Err(error), _) => Err(TaskEngineError::Broker(error)),
        }
    }
}

/// 由单个任务自身页面内容引起的失败：不得阻塞其他任务。
fn is_page_rejection(error: &TaskEngineError) -> bool {
    matches!(
        error,
        TaskEngineError::Projection(_)
            | TaskEngineError::NotFound("projection task")
            | TaskEngineError::Json(_)
    )
}

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    fn verify_vault_binding(&self) -> Result<(), TaskEngineError> {
        if self.broker.vault_uuid() != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker and Control DB belong to different Vaults".to_owned(),
            ));
        }
        Ok(())
    }

    fn capture_inputs(
        &self,
        task_id: TaskId,
        spec: &TaskSpec,
    ) -> Result<Snapshot, TaskEngineError> {
        let mut paths = spec.input_paths.clone();
        paths.push(task_page_path(task_id)?);
        paths.sort();
        paths.dedup();
        let mut inputs = Vec::with_capacity(paths.len());
        for path in paths {
            let object = self.broker.read(&path).map_err(TaskEngineError::Broker)?;
            inputs.push(SnapshotInput {
                path,
                expected_revision: object.revision,
            });
        }
        let snapshot = self
            .broker
            .snapshot(&inputs)
            .map_err(TaskEngineError::Broker)?;
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker snapshot belongs to another Vault".to_owned(),
            ));
        }
        Ok(snapshot)
    }

    fn inputs_unchanged(
        &self,
        task: &TaskRecord,
        snapshot: &Snapshot,
    ) -> Result<bool, TaskEngineError> {
        let page = task_page_path(task.task_id)?;
        let comparison = json!({
            "status": "queued", "active_attempt": null, "last_attempt": null, "input_snapshot": null,
        });
        for input in &snapshot.files {
            let current = match self.broker.read(&input.path) {
                Ok(value) => value,
                Err(BrokerError::NotFound(_)) => return Ok(false),
                Err(error) => return Err(TaskEngineError::Broker(error)),
            };
            if input.path == page {
                projection::validate_task_page(&current.content, task.task_id)
                    .map_err(TaskEngineError::Projection)?;
                let prior = projection::apply_projection(&input.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                let current = projection::apply_projection(&current.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                if current != prior {
                    return Ok(false);
                }
            } else if current.revision != input.revision {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn roll_forward(
        &mut self,
        attempt: &AttemptIdentity,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<ArtifactCommitReceipt, TaskEngineError> {
        let plan = self.staged_plan(attempt, reservation)?;
        let stopped = if reservation
            .report_json
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            == Some("stopped")
        {
            Some(self.stopped_receipt_from_plan(attempt, &plan)?)
        } else {
            None
        };
        let subject = AuditSubject::default()
            .with_task(attempt.task_id)
            .with_attempt(attempt.attempt_id);
        let intent_payload = SanitizedPayload::try_from(json!({
            "fence": attempt.fence.get(),
            "artifact_count": plan.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id.clone(),
                    AuditPhase::Intent,
                    "task_engine.report",
                    intent_payload,
                )
                .with_subject(subject),
            )
            .map_err(map_audit)?;

        let commit = ArtifactCommit {
            task_id: attempt.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            audit_intent: plan.event_id.to_string(),
            idempotency_key: format!("task-engine:{}:{}", attempt.attempt_id, plan.event_id),
            artifacts: plan.artifacts,
        };
        let receipt = self
            .broker
            .commit_attempt_artifacts(commit)
            .map_err(TaskEngineError::CommitFailed)?;

        let encoded_receipt = serde_json::to_value(&receipt)?;
        if let Some(stopped) = stopped {
            self.finish_stopped_candidates(attempt, stopped, &receipt, now_ms)?;
        } else {
            self.control_db
                .finalize_terminal(
                    attempt.task_id,
                    attempt.attempt_id,
                    attempt.fence,
                    TaskStatus::NeedsReview,
                    Some(&encoded_receipt),
                    now_ms,
                )
                .map_err(map_db)?;
        }
        // 只有在权威 T2 提交之后，审计才能宣称工作流状态。
        // T2 的 outbox 能扛过恰好发生在此边界的崩溃或账本不可用。
        self.deliver_pending_outcomes(Some(attempt.attempt_id), now_ms)?;
        Ok(receipt)
    }

    fn deliver_pending_outcomes(
        &mut self,
        only_attempt: Option<AttemptId>,
        now_ms: i64,
    ) -> Result<usize, TaskEngineError> {
        let pending = self.control_db.pending_terminal_audits().map_err(map_db)?;
        let mut delivered = 0;
        for outcome in pending {
            if only_attempt.is_some_and(|id| id != outcome.attempt.attempt_id) {
                continue;
            }
            self.deliver_outcome(&outcome)?;
            self.control_db
                .mark_terminal_audit_delivered(outcome.attempt.attempt_id, now_ms)
                .map_err(map_db)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn deliver_outcome(&self, outcome: &PendingTerminalAudit) -> Result<(), TaskEngineError> {
        let reservation = outcome.attempt.terminal.as_ref().ok_or_else(|| {
            TaskEngineError::Integrity("pending audit has no report reservation".to_owned())
        })?;
        let plan = self.staged_plan(&prior_identity(&outcome.attempt), reservation)?;
        let receipt: ArtifactCommitReceipt = serde_json::from_value(outcome.receipt_json.clone())?;
        let payload = SanitizedPayload::try_from(json!({
            "status": outcome.status,
            "manifest_path": receipt.manifest_path,
            "manifest_revision": receipt.manifest_revision,
            "artifact_count": receipt.artifacts.len(),
        }))
        .map_err(map_audit)?;
        self.audit_ledger
            .append(
                AuditEvent::new(
                    plan.event_id,
                    AuditPhase::Outcome,
                    "task_engine.report",
                    payload,
                )
                .with_subject(
                    AuditSubject::default()
                        .with_task(outcome.attempt.task_id)
                        .with_attempt(outcome.attempt.attempt_id),
                ),
            )
            .map_err(map_audit)?;
        Ok(())
    }
}

fn task_page_path(task_id: TaskId) -> Result<VaultPath, TaskEngineError> {
    VaultPath::parse(format!("ops/tasks/{task_id}.md"))
        .map_err(|error| TaskEngineError::Integrity(error.to_string()))
}

fn snapshot_record(snapshot: &Snapshot) -> SnapshotRecord {
    SnapshotRecord {
        snapshot_id: snapshot.id.as_str().to_owned(),
        vault_uuid: snapshot.vault_uuid,
        entries: snapshot
            .files
            .iter()
            .map(|file| SnapshotEntryRecord {
                path: file.path.to_string(),
                revision: file.revision.to_string(),
                effective_label_json: json!({"classification": "restricted"}),
            })
            .collect(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedReport {
    event_id: AuditEventId,
    report: RunnerReport,
}

struct CommitPlan {
    event_id: AuditEventId,
    artifacts: Vec<ArtifactCandidate>,
}

#[derive(Clone, Copy)]
struct AttemptIdentity {
    task_id: TaskId,
    attempt_id: AttemptId,
    fence: Fence,
}

fn prior_identity(attempt: &AttemptRecord) -> AttemptIdentity {
    AttemptIdentity {
        task_id: attempt.task_id,
        attempt_id: attempt.attempt_id,
        fence: attempt.fence,
    }
}

fn decode_plan(
    attempt: &AttemptIdentity,
    reservation: &TerminalReservation,
) -> Result<CommitPlan, TaskEngineError> {
    let persisted: PersistedReport = serde_json::from_value(reservation.report_json.clone())?;
    let artifacts: Vec<ArtifactCandidate> =
        serde_json::from_value(reservation.artifacts_json.clone())?;
    if persisted.report.task_id != attempt.task_id
        || persisted.report.attempt_id != attempt.attempt_id
        || persisted.report.fence != attempt.fence
    {
        return Err(TaskEngineError::Integrity(
            "reserved report identity does not match attempt".to_owned(),
        ));
    }
    let declared: Vec<_> = persisted
        .report
        .artifacts
        .iter()
        .map(|artifact| ArtifactCandidate {
            name: artifact.name.clone(),
            content: artifact.content.clone(),
        })
        .collect();
    if declared != artifacts {
        return Err(TaskEngineError::Integrity(
            "reserved artifacts do not match runner report".to_owned(),
        ));
    }
    Ok(CommitPlan {
        event_id: persisted.event_id,
        artifacts,
    })
}

fn task_view(record: &TaskRecord) -> Result<TaskView, TaskEngineError> {
    Ok(TaskView {
        task_id: record.task_id,
        spec: decode_spec(record)?,
        status: record.status,
        active_attempt: record.active_attempt,
        last_attempt: record.last_attempt,
        input_snapshot: record.input_snapshot.clone(),
    })
}

fn decode_spec(record: &TaskRecord) -> Result<TaskSpec, TaskEngineError> {
    let spec: TaskSpec = serde_json::from_value(record.definition_json.clone())?;
    spec.validate_board()?;
    Ok(spec)
}

/// 投影载荷里的 task 状态：载荷是 Control DB 写出的 JSON，按公开枚举解析而不比较状态名字符串。
fn projected_status(payload: &serde_json::Value) -> Option<TaskStatus> {
    serde_json::from_value(payload.get("status")?.clone()).ok()
}

fn map_db(error: DbError) -> TaskEngineError {
    match error {
        DbError::NotFound(record) => TaskEngineError::NotFound(record),
        // 终态判断只认 Control DB 的结构化变体，不解释状态名字符串（契约 §6）。
        DbError::AlreadyTerminal { .. } => TaskEngineError::AlreadyTerminal,
        DbError::InvalidTransition { from, operation } => {
            TaskEngineError::InvalidTransition { from, operation }
        }
        DbError::FenceMismatch => TaskEngineError::StaleFence,
        DbError::IdempotencyMismatch => TaskEngineError::IdempotencyMismatch,
        DbError::PathsHeld { by_instance, paths } => {
            TaskEngineError::PathsHeld { by_instance, paths }
        }
        other => TaskEngineError::ControlDb(other),
    }
}

fn map_audit(error: AuditError) -> TaskEngineError {
    match error {
        AuditError::Unavailable => TaskEngineError::AuditUnavailable,
        AuditError::IdempotencyConflict { .. } => TaskEngineError::IdempotencyMismatch,
        other => TaskEngineError::Audit(other),
    }
}
