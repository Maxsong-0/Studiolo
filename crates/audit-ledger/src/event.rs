use crate::{AuditError, SanitizedPayload};
use serde::{Deserialize, Serialize};
use std::fmt;
use studiolo_core::{AttemptId, TaskId, ToolId};

/// 同一被审计操作的 Intent 与 Outcome 共用的稳定标识。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AuditEventId(String);

impl AuditEventId {
    /// 解析事件标识：空串、仅含空白或含控制字符的值一律拒绝。
    pub fn parse(value: impl Into<String>) -> Result<Self, AuditError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(AuditError::InvalidEventId);
        }
        Ok(Self(value))
    }

    /// 返回标识文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuditEventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for AuditEventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// 可能产生副作用的操作的两个持久化阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPhase {
    /// 在敏感操作开始之前持久化。
    Intent,
    /// 在得出放行、拒绝、失败或无法确定的结果之后持久化。
    Outcome,
}

/// 过滤审计记录用的主体元数据，过滤时无需检视 payload。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSubject {
    task_id: Option<TaskId>,
    attempt_id: Option<AttemptId>,
    tool: Option<ToolId>,
}

impl AuditSubject {
    /// 为该主体关联任务。
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// 为该主体关联 attempt。
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// 为该主体关联逻辑工具。
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// 返回任务标识；未设置时为 `None`。
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// 返回 attempt 标识；未设置时为 `None`。
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// 返回逻辑工具；未设置时为 `None`。
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }
}

/// 提交给 [`crate::AuditLedger::append`] 的一条语义审计事件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    event_id: AuditEventId,
    phase: AuditPhase,
    action: String,
    subject: AuditSubject,
    payload: SanitizedPayload,
}

impl AuditEvent {
    /// 创建主体为空的事件。
    #[must_use]
    pub fn new(
        event_id: AuditEventId,
        phase: AuditPhase,
        action: impl Into<String>,
        payload: SanitizedPayload,
    ) -> Self {
        Self {
            event_id,
            phase,
            action: action.into(),
            subject: AuditSubject::default(),
            payload,
        }
    }

    /// 替换事件可供过滤的主体元数据。
    #[must_use]
    pub fn with_subject(mut self, subject: AuditSubject) -> Self {
        self.subject = subject;
        self
    }

    /// 返回稳定的事件标识。
    #[must_use]
    pub fn event_id(&self) -> &AuditEventId {
        &self.event_id
    }

    /// 返回操作阶段。
    #[must_use]
    pub fn phase(&self) -> AuditPhase {
        self.phase
    }

    /// 返回稳定的动作名。
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// 返回事件主体。
    #[must_use]
    pub fn subject(&self) -> &AuditSubject {
        &self.subject
    }

    /// 返回已通过检查的 payload。
    #[must_use]
    pub fn payload(&self) -> &SanitizedPayload {
        &self.payload
    }
}
