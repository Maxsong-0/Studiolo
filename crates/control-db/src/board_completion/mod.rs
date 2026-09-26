//! 黑板回执的 fence 校验、终态仲裁与编号预留。
mod handoff;
mod legacy;
mod numbering;
mod receipt;
use crate::TerminalReservation;
use serde_json::Value;

pub(super) fn is_pointer_reservation(reservation: &TerminalReservation) -> bool {
    let Some(pointer) = reservation.artifacts_json.as_object() else {
        return false;
    };
    let Some(metadata) = reservation.report_json.as_object() else {
        return false;
    };
    pointer.len() == 2
        && ["id", "sha256"].iter().all(|key| {
            pointer
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        && metadata.len() <= 7
        && metadata.iter().all(|(key, value)| {
            matches!(
                key.as_str(),
                "format" | "event_id" | "task_id" | "attempt_id" | "fence" | "outcome"
            ) && (value.is_number() || value.as_str().is_some_and(|s| s.len() <= 256))
        })
}
