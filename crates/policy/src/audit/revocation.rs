//! 吊销事件族：三级吊销、全局吊销解除与类型吊销随能力集重新发布而解除。

use serde_json::json;
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditPhase};

use crate::RevocationLevel;

use super::{
    GLOBAL_RELEASED, GLOBAL_REVOKED, INSTANCE_REVOKED, Principal, TYPE_RESTORED, TYPE_REVOKED,
    event, fresh_id,
};

/// 三级吊销事件（按级别选动作名）。
pub fn revoked(
    principal: Principal<'_>,
    level: RevocationLevel,
    target: &str,
    reason: &str,
) -> Result<AuditEvent, AuditError> {
    let action = match level {
        RevocationLevel::Instance => INSTANCE_REVOKED,
        RevocationLevel::Type => TYPE_REVOKED,
        RevocationLevel::Global => GLOBAL_REVOKED,
    };
    event(
        fresh_id(action)?,
        AuditPhase::Outcome,
        action,
        principal,
        json!({"level": level.as_str(), "target": target, "reason": reason}),
    )
}

/// `agent.global_released`。
pub fn global_released(principal: Principal<'_>) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(GLOBAL_RELEASED)?,
        AuditPhase::Outcome,
        GLOBAL_RELEASED,
        principal,
        json!({}),
    )
}

/// `agent_type.restored`：类型吊销随能力集重新发布而解除。
pub fn type_restored(
    principal: Principal<'_>,
    ai_type: &str,
    version: u64,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(TYPE_RESTORED)?,
        AuditPhase::Outcome,
        TYPE_RESTORED,
        principal,
        json!({"target": ai_type, "version": version}),
    )
}
