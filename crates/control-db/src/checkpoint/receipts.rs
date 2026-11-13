//! 逐条扫描持久 ACK；同一事件构造被运行时投递复用。
use crate::{
    RuntimeAuditRecord, TaskStatus, TerminalReservation, audit_events,
    types::{parse_attempt_id, parse_task_id},
};
use rusqlite::Connection;
use studiolo_audit_ledger::{FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::Error;

pub(super) fn verify(db: &Connection, audit: &FrozenAudit) -> Result<ReceiptCoverage, Error> {
    let mut coverage = ReceiptCoverage::default();
    let mut statement = db.prepare("SELECT CASE WHEN octet_length(event_id) BETWEEN 1 AND 88 THEN event_id END,CASE WHEN octet_length(action) BETWEEN 1 AND 256 THEN action END,CASE WHEN octet_length(payload_json) BETWEEN 1 AND 8192 THEN payload_json END FROM runtime_audit_outbox WHERE delivered=1 ORDER BY sequence").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let payload: String = row.get(2).map_err(invalid)?;
        let event = RuntimeAuditRecord {
            event_id: row.get(0).map_err(invalid)?,
            action: row.get(1).map_err(invalid)?,
            payload: serde_json::from_str(&payload).map_err(invalid)?,
        }
        .audit_event()
        .map_err(invalid)?;
        coverage
            .include(audit.verify_event(&event).map_err(invalid)?)
            .map_err(invalid)?;
    }
    let mut statement = db.prepare("SELECT a.task_id,a.attempt_id,o.status,CASE WHEN octet_length(a.terminal_json) BETWEEN 1 AND 16384 THEN a.terminal_json END,CASE WHEN octet_length(c.receipt_json) BETWEEN 1 AND 1048576 THEN c.receipt_json END FROM terminal_audit_outbox o JOIN attempts a USING(attempt_id) JOIN artifact_commits c USING(attempt_id) WHERE o.delivered_at_ms IS NOT NULL ORDER BY o.attempt_id").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let task = parse_task_id(&row.get::<_, String>(0).map_err(invalid)?).map_err(invalid)?;
        let attempt =
            parse_attempt_id(&row.get::<_, String>(1).map_err(invalid)?).map_err(invalid)?;
        let status =
            TaskStatus::parse(&row.get::<_, String>(2).map_err(invalid)?).map_err(invalid)?;
        let reservation: TerminalReservation =
            serde_json::from_str(&row.get::<_, String>(3).map_err(invalid)?).map_err(invalid)?;
        let receipt =
            serde_json::from_str(&row.get::<_, String>(4).map_err(invalid)?).map_err(invalid)?;
        let event =
            audit_events::terminal_event(task, attempt, status, &reservation.report_json, &receipt)
                .map_err(invalid)?;
        coverage
            .include(audit.verify_event(&event).map_err(invalid)?)
            .map_err(invalid)?;
    }
    Ok(coverage)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
