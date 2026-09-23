//! 同步、对象安全的任务生命周期接缝 `TaskEngine`。
use super::*;

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
