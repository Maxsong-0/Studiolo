//! 运行时与备份核对共用的纯事件构造；不调用账本、不持有外部连接。
use crate::{DbError, PendingTerminalAudit, RuntimeAuditRecord, TaskStatus};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, AuditSubject, SanitizedPayload};
use studiolo_core::{AttemptId, Revision, TaskId, VaultPath};

impl RuntimeAuditRecord {
    /// 与实际投递完全相同的原事件；供冻结后核对持久 ACK。
    pub fn audit_event(&self) -> Result<AuditEvent, DbError> {
        let mut subject = AuditSubject::default();
        if let Some(task) = self.payload.get("task_id") {
            subject = subject.with_task(serde_json::from_value(task.clone())?);
        }
        if let Some(attempt) = self.payload.get("attempt_id") {
            subject = subject.with_attempt(serde_json::from_value(attempt.clone())?);
        }
        Ok(AuditEvent::new(
            AuditEventId::parse(format!("runtime:{}", self.event_id)).map_err(invalid)?,
            AuditPhase::Outcome,
            &self.action,
            SanitizedPayload::try_from(self.payload.clone()).map_err(invalid)?,
        )
        .with_subject(subject))
    }
}
impl PendingTerminalAudit {
    /// 从既有终态元数据重建投递事件；TaskEngine 仍须先验证 Broker staging。
    pub fn audit_event(&self) -> Result<AuditEvent, DbError> {
        let reservation = self.attempt.terminal.as_ref().ok_or_else(|| invalid(()))?;
        terminal_event(
            self.attempt.task_id,
            self.attempt.attempt_id,
            self.status,
            &reservation.report_json,
            &self.receipt_json,
        )
    }
}
pub(super) fn terminal_event(
    task: TaskId,
    attempt: AttemptId,
    status: TaskStatus,
    report: &Value,
    receipt: &Value,
) -> Result<AuditEvent, DbError> {
    let id = report
        .get("event_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(()))?;
    let path: VaultPath = serde_json::from_value(
        receipt
            .get("manifest_path")
            .cloned()
            .ok_or_else(|| invalid(()))?,
    )?;
    let revision: Revision = serde_json::from_value(
        receipt
            .get("manifest_revision")
            .cloned()
            .ok_or_else(|| invalid(()))?,
    )?;
    let count = receipt
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(()))?
        .len();
    Ok(AuditEvent::new(AuditEventId::parse(id).map_err(invalid)?, AuditPhase::Outcome, "task_engine.report",
        SanitizedPayload::try_from(json!({"status": status, "manifest_path": path, "manifest_revision": revision, "artifact_count": count})).map_err(invalid)?)
        .with_subject(AuditSubject::default().with_task(task).with_attempt(attempt)))
}
fn invalid<T>(_: T) -> DbError {
    DbError::Corrupt("控制库审计事件不合法".into())
}
