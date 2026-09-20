//! 封闭审计事件的规范字段集合，不接纳任意自由文本或错桶计数。

use super::SessionError;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

pub(super) fn payload(
    event: &AuditEvent,
    kind: &str,
    bucket: Option<i64>,
    through: Option<i64>,
    namespace: &str,
) -> Result<(), SessionError> {
    let value = event.payload().as_value();
    let object = value.as_object().ok_or(SessionError::CorruptStore)?;
    let id = event.event_id().as_str();
    let valid = match kind {
        "issue" => {
            event.action() == "session.issue"
                && event.phase() == AuditPhase::Outcome
                && object.len() == 6
                && value["decision"] == "allow"
                && value["publication"] == "not_released"
                && value["identity_version"].as_i64().is_some_and(|v| v > 0)
                && value["created_at_ms"].as_i64().is_some_and(|v| v >= 0)
                && value["absolute_expires_at_ms"]
                    .as_i64()
                    .is_some_and(|v| v > 0)
                && value["session_id"]
                    .as_str()
                    .is_some_and(|session| id == format!("{namespace}issue:{session}"))
        }
        "revoke" if value["scope"] == "restore" => super::restore::valid_payload(event, namespace),
        "revoke" => {
            event.action() == "session.revoked"
                && object.len() == 5
                && value["effective"] == "already_revoked"
                && value["revoked"]
                    .as_u64()
                    .is_some_and(|v| (1..=1024).contains(&v))
                && value["identity_version"].as_i64().is_some_and(|v| v > 0)
                && ((value["scope"] == "one"
                    && value["target"]
                        .as_str()
                        .is_some_and(|target| id == format!("{namespace}revoke:{target}")))
                    || (value["scope"] == "all"
                        && value["target"] == "all"
                        && id == format!("{namespace}revoke-all:{}", value["identity_version"])))
        }
        "expire" => {
            event.action() == "session.expired"
                && event.phase() == AuditPhase::Outcome
                && object.len() == 4
                && matches!(value["reason"].as_str(), Some("idle" | "absolute"))
                && matches!(value["state"].as_str(), Some("expired" | "abandoned"))
                && value["identity_version"].as_i64().is_some_and(|v| v > 0)
                && value["session_id"]
                    .as_str()
                    .is_some_and(|session| id == format!("{namespace}expire:{session}"))
        }
        "denied" => {
            event.action() == "session.denied"
                && event.phase() == AuditPhase::Outcome
                && object.len() == 3
                && bucket.is_some_and(|v| (0..8).contains(&v))
                && through.is_some_and(|v| v > 0)
                && value["through_count"].as_i64() == through
                && value["after_count"]
                    .as_i64()
                    .is_some_and(|v| v >= 0 && Some(v) < through)
                && id
                    == format!(
                        "{namespace}denied:{}:{}",
                        bucket.unwrap_or(-1),
                        through.unwrap_or(-1)
                    )
                && value["reason"].as_str()
                    == bucket.and_then(|index| {
                        [
                            "identity",
                            "access_expired",
                            "unknown_session",
                            "csrf",
                            "expired",
                            "clock",
                            "issue",
                            "capacity",
                        ]
                        .get(index as usize)
                        .copied()
                    })
        }
        _ => false,
    };
    if !valid || (kind != "denied" && (bucket.is_some() || through.is_some())) {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}
