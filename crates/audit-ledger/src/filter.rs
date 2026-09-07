use crate::{AuditEventId, AuditPhase, AuditRecord};
use studiolo_core::{AttemptId, TaskId, ToolId};

/// Public query constraints. Unset fields do not restrict results.
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
    /// Filters by event identifier.
    #[must_use]
    pub fn with_event_id(mut self, event_id: AuditEventId) -> Self {
        self.event_id = Some(event_id);
        self
    }

    /// Filters by task identifier.
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Filters by attempt identifier.
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// Filters by tool identifier.
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// Filters by phase.
    #[must_use]
    pub fn with_phase(mut self, phase: AuditPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// Filters by exact stable action name.
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Returns records whose offset is strictly greater than `offset`.
    #[must_use]
    pub fn after_offset(mut self, offset: u64) -> Self {
        self.after_offset = Some(offset);
        self
    }

    /// Limits the number of records returned in append order.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Returns the event identifier constraint.
    #[must_use]
    pub fn event_id(&self) -> Option<&AuditEventId> {
        self.event_id.as_ref()
    }

    /// Returns the task constraint.
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// Returns the attempt constraint.
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// Returns the logical tool constraint.
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }

    /// Returns the phase constraint.
    #[must_use]
    pub fn phase(&self) -> Option<AuditPhase> {
        self.phase
    }

    /// Returns the exact action constraint.
    #[must_use]
    pub fn action(&self) -> Option<&str> {
        self.action.as_deref()
    }

    /// Returns the exclusive lower offset bound.
    #[must_use]
    pub fn offset_exclusive(&self) -> Option<u64> {
        self.after_offset
    }

    /// Returns the maximum number of records requested.
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
