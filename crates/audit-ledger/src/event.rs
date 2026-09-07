use crate::{AuditError, SanitizedPayload};
use serde::{Deserialize, Serialize};
use std::fmt;
use studiolo_core::{AttemptId, TaskId, ToolId};

/// Stable identity shared by the intent and outcome of one audited operation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AuditEventId(String);

impl AuditEventId {
    /// Parses a non-empty identifier without control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, AuditError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(AuditError::InvalidEventId);
        }
        Ok(Self(value))
    }

    /// Returns the textual identifier.
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

/// The two durable phases of an operation with possible side effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPhase {
    /// Persisted before the sensitive operation starts.
    Intent,
    /// Persisted after an allow, denial, failure, or indeterminate result.
    Outcome,
}

/// Subject metadata used to filter audit records without inspecting payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSubject {
    task_id: Option<TaskId>,
    attempt_id: Option<AttemptId>,
    tool: Option<ToolId>,
}

impl AuditSubject {
    /// Associates a task with this subject.
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Associates an attempt with this subject.
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// Associates a logical tool with this subject.
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// Returns the task identifier, if present.
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// Returns the attempt identifier, if present.
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// Returns the logical tool, if present.
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }
}

/// One semantic audit event supplied to [`crate::AuditLedger::append`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    event_id: AuditEventId,
    phase: AuditPhase,
    action: String,
    subject: AuditSubject,
    payload: SanitizedPayload,
}

impl AuditEvent {
    /// Creates an event with an empty subject.
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

    /// Replaces the event's filterable subject metadata.
    #[must_use]
    pub fn with_subject(mut self, subject: AuditSubject) -> Self {
        self.subject = subject;
        self
    }

    /// Returns the stable event identifier.
    #[must_use]
    pub fn event_id(&self) -> &AuditEventId {
        &self.event_id
    }

    /// Returns the operation phase.
    #[must_use]
    pub fn phase(&self) -> AuditPhase {
        self.phase
    }

    /// Returns the stable action name.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the event subject.
    #[must_use]
    pub fn subject(&self) -> &AuditSubject {
        &self.subject
    }

    /// Returns the checked payload.
    #[must_use]
    pub fn payload(&self) -> &SanitizedPayload {
        &self.payload
    }
}
