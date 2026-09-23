//! 人写的任务定义与 M0 封闭命令集，以及 TaskEngine 接缝的请求、结果与视图值类型。
use super::*;

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
    pub(super) fn validate(&self) -> Result<(), TaskEngineError> {
        if self.adapter != "stub" {
            return Err(TaskEngineError::InvalidTaskSpec(
                "M0 only enables the in-process stub adapter",
            ));
        }
        self.validate_board()
    }

    pub(super) fn validate_board(&self) -> Result<(), TaskEngineError> {
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

pub(super) fn task_view(record: &TaskRecord) -> Result<TaskView, TaskEngineError> {
    Ok(TaskView {
        task_id: record.task_id,
        spec: decode_spec(record)?,
        status: record.status,
        active_attempt: record.active_attempt,
        last_attempt: record.last_attempt,
        input_snapshot: record.input_snapshot.clone(),
    })
}

pub(super) fn decode_spec(record: &TaskRecord) -> Result<TaskSpec, TaskEngineError> {
    let spec: TaskSpec = serde_json::from_value(record.definition_json.clone())?;
    spec.validate_board()?;
    Ok(spec)
}
