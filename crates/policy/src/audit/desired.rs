//! desired 事件族：提交与复核；payload 只含类别、subject、哈希与校验结论。

use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditPhase};

use crate::DesiredRecord;

use super::{DESIRED_SUBMITTED, Principal, VALIDATED, event, fresh_id};

fn desired_payload(record: &DesiredRecord) -> Value {
    json!({
        "desired_id": record.id,
        "kind": record.kind.as_str(),
        "subject": record.subject,
        "body_sha256": record.body_sha256,
        "status": record.status.as_str(),
        "ok": record.report.as_ref().is_some_and(|report| report.ok),
        "expansions": record.report.as_ref().map_or(0, |report| report.expansions.len()),
        "base_effective_hash": record.report.as_ref().map(|report| report.base_effective_hash.clone()),
    })
}

/// `policy.desired_submitted`。
pub fn desired_submitted(
    principal: Principal<'_>,
    record: &DesiredRecord,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(DESIRED_SUBMITTED)?,
        AuditPhase::Outcome,
        DESIRED_SUBMITTED,
        principal,
        desired_payload(record),
    )
}

/// `policy.validated`。
pub fn validated(
    principal: Principal<'_>,
    record: &DesiredRecord,
) -> Result<AuditEvent, AuditError> {
    event(
        fresh_id(VALIDATED)?,
        AuditPhase::Outcome,
        VALIDATED,
        principal,
        desired_payload(record),
    )
}
