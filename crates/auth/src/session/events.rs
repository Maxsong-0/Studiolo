//! 只从封闭字段建立不可变审计事件；不接收原始请求或凭证摘要。

use super::{SessionError, SessionId};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

pub(super) struct Grant {
    pub store_id: String,
    pub id: SessionId,
    pub epoch: i64,
    pub created: i64,
    pub absolute: i64,
    pub access_expiry: i64,
}

impl Grant {
    pub fn event(&self, phase: AuditPhase) -> Result<AuditEvent, SessionError> {
        event(
            &format!("{}:issue:{}", self.store_id, self.id.as_str()),
            phase,
            "session.issue",
            json!({
                "session_id":self.id.as_str(), "identity_version":self.epoch,
                "created_at_ms":self.created, "absolute_expires_at_ms":self.absolute,
                "decision":if phase==AuditPhase::Outcome {"allow"} else {"requested"},
                "publication":"not_released"
            }),
        )
    }
}

pub(super) fn event(
    id: &str,
    phase: AuditPhase,
    action: &str,
    payload: Value,
) -> Result<AuditEvent, SessionError> {
    Ok(AuditEvent::new(
        AuditEventId::parse(id).map_err(|_| SessionError::CorruptStore)?,
        phase,
        action,
        SanitizedPayload::try_from(payload).map_err(|_| SessionError::CorruptStore)?,
    ))
}

pub(super) fn phase(phase: AuditPhase) -> &'static str {
    match phase {
        AuditPhase::Intent => "intent",
        AuditPhase::Outcome => "outcome",
    }
}

pub(super) fn encode(event: &AuditEvent) -> Result<String, SessionError> {
    let encoded = serde_json::to_string(event).map_err(|_| SessionError::CorruptStore)?;
    if encoded.len() > super::schema::MAX_EVENT_BYTES as usize {
        return Err(SessionError::Capacity);
    }
    let decoded: AuditEvent =
        serde_json::from_str(&encoded).map_err(|_| SessionError::CorruptStore)?;
    if &decoded != event {
        return Err(SessionError::CorruptStore);
    }
    Ok(encoded)
}
