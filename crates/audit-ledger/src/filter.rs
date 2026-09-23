use crate::{AuditEventId, AuditPhase, AuditRecord};
use studiolo_core::{AttemptId, TaskId, ToolId};

/// 公开的查询约束；未设置的字段不限制结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditFilter {
    event_id: Option<AuditEventId>,
    task_id: Option<TaskId>,
    attempt_id: Option<AttemptId>,
    tool: Option<ToolId>,
    phase: Option<AuditPhase>,
    action: Option<String>,
    after_offset: Option<u64>,
    limit: Option<usize>,
}

impl AuditFilter {
    /// 按事件标识过滤。
    #[must_use]
    pub fn with_event_id(mut self, event_id: AuditEventId) -> Self {
        self.event_id = Some(event_id);
        self
    }

    /// 按任务标识过滤。
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// 按 attempt 标识过滤。
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// 按工具标识过滤。
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// 按阶段过滤。
    #[must_use]
    pub fn with_phase(mut self, phase: AuditPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// 按稳定动作名精确匹配过滤。
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// 只返回 offset 严格大于 `offset` 的记录。
    #[must_use]
    pub fn after_offset(mut self, offset: u64) -> Self {
        self.after_offset = Some(offset);
        self
    }

    /// 限制返回条数：按追加顺序取前若干条命中记录。
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 返回事件标识约束。
    #[must_use]
    pub fn event_id(&self) -> Option<&AuditEventId> {
        self.event_id.as_ref()
    }

    /// 返回任务约束。
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// 返回 attempt 约束。
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// 返回逻辑工具约束。
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }

    /// 返回阶段约束。
    #[must_use]
    pub fn phase(&self) -> Option<AuditPhase> {
        self.phase
    }

    /// 返回精确匹配的动作约束。
    #[must_use]
    pub fn action(&self) -> Option<&str> {
        self.action.as_deref()
    }

    /// 返回 offset 下界（不含边界值本身）。
    #[must_use]
    pub fn offset_exclusive(&self) -> Option<u64> {
        self.after_offset
    }

    /// 返回请求的最大记录条数。
    #[must_use]
    pub fn limit(&self) -> Option<usize> {
        self.limit
    }
}

impl AuditFilter {
    pub(crate) fn matches(&self, record: &AuditRecord) -> bool {
        let event = record.event();
        self.event_id
            .as_ref()
            .is_none_or(|value| value == event.event_id())
            && self
                .task_id
                .is_none_or(|value| Some(value) == event.subject().task_id())
            && self
                .attempt_id
                .is_none_or(|value| Some(value) == event.subject().attempt_id())
            && self
                .tool
                .as_ref()
                .is_none_or(|value| Some(value) == event.subject().tool())
            && self.phase.is_none_or(|value| value == event.phase())
            && self
                .action
                .as_deref()
                .is_none_or(|value| value == event.action())
            && self
                .after_offset
                .is_none_or(|value| record.offset() > value)
    }
}
