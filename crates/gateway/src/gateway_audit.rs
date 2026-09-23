//! MemoryGateway 的审计接线：审计事件 ID 派生、审计追加与拒绝审计。

use crate::{CallId, GatewayError, JobCapability, MemoryGateway};
use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_core::ToolId;

impl MemoryGateway {
    pub(crate) fn audit_event_id(
        &self,
        domain: &str,
        capability: &JobCapability,
        call_id: &CallId,
    ) -> Result<AuditEventId, GatewayError> {
        AuditEventId::parse(format!(
            "{domain}:task={}:attempt={}:fence={}:call={}:{}",
            capability.task_id(),
            capability.attempt_id(),
            capability.fence().get(),
            call_id.as_str().len(),
            call_id,
        ))
        .map_err(|_| GatewayError::InvalidCallId)
    }

    /// 拒绝与冲突审计的独立事件 ID：`{base}:deny:{reason}`。
    /// 同一调用因不同原因被多次拒绝时各留一条记录，且永不与基础 ID 上的 Intent/Outcome 冲突。
    pub(crate) fn denial_audit_event_id(
        &self,
        base: &AuditEventId,
        error: &GatewayError,
    ) -> Result<AuditEventId, GatewayError> {
        AuditEventId::parse(format!("{base}:deny:{}", error.audit_reason()))
            .map_err(|_| GatewayError::InvalidCallId)
    }

    pub(crate) fn append_audit(
        &self,
        event_id: AuditEventId,
        phase: AuditPhase,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        payload: Value,
    ) -> Result<(), GatewayError> {
        let payload =
            SanitizedPayload::try_from(payload).map_err(|_| GatewayError::AuditUnavailable)?;
        let subject = AuditSubject::default()
            .with_task(capability.task_id())
            .with_attempt(capability.attempt_id())
            .with_tool(tool);
        self.audit
            .append(AuditEvent::new(event_id, phase, action, payload).with_subject(subject))
            .map(|_| ())
            .map_err(|error| match error {
                // 同一事件 ID 被不同内容复用：这是调用方的 call ID 冲突，不是账本不可用。
                AuditError::IdempotencyConflict { .. } => GatewayError::CallConflict,
                _ => GatewayError::AuditUnavailable,
            })
    }

    pub(crate) fn audit_denial(
        &self,
        event_id: AuditEventId,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        call_id: &CallId,
        error: GatewayError,
    ) -> GatewayError {
        let audit_result = self.append_audit(
            event_id,
            AuditPhase::Outcome,
            action,
            capability,
            tool.clone(),
            json!({
                "call_id": call_id.as_str(),
                "tool": tool.as_str(),
                "fence": capability.fence().get(),
                "decision": "deny",
                "reason": error.audit_reason(),
            }),
        );
        audit_result.err().unwrap_or(error)
    }

    /// 拒绝准入：为当前拒绝原因派生独立审计 ID 并追加 deny outcome；ID 派生失败按其错误返回。
    pub(crate) fn deny_admission(
        &self,
        error: GatewayError,
        audit_id: &AuditEventId,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        call_id: &CallId,
    ) -> GatewayError {
        match self.denial_audit_event_id(audit_id, &error) {
            Ok(denial_id) => self.audit_denial(denial_id, action, capability, tool, call_id, error),
            Err(id_error) => id_error,
        }
    }
}
