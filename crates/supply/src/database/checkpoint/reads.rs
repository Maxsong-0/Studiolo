//! 派生索引虽可重建，封存时也不能把错 offset 的索引当作原审计确认。
use rusqlite::Connection;
use studiolo_audit_ledger::{AuditPhase, FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::Error;

pub(super) fn verify(db: &Connection, audit: &FrozenAudit) -> Result<ReceiptCoverage, Error> {
    let mut coverage = ReceiptCoverage::default();
    let mut statement=db.prepare("SELECT ledger_offset,recorded_at_ms,CASE WHEN octet_length(recorded_at) BETWEEN 1 AND 64 THEN recorded_at END,CASE WHEN octet_length(type_name) BETWEEN 1 AND 32 THEN type_name END,CASE WHEN octet_length(instance_id)=26 THEN instance_id END,CASE WHEN octet_length(path) BETWEEN 1 AND 4096 THEN path END,CASE WHEN octet_length(revision) BETWEEN 1 AND 128 THEN revision END,CASE WHEN octet_length(tool) BETWEEN 1 AND 128 THEN tool END FROM reads ORDER BY seq").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let offset: u64 = u64::try_from(row.get::<_, i64>(0).map_err(invalid)?).map_err(invalid)?;
        let record = audit.read_record(offset).map_err(invalid)?;
        if record.event().action() != crate::audit::CONTENT_READ
            || record.event().phase() != AuditPhase::Outcome
            || record.recorded_at().as_millisecond() != row.get::<_, i64>(1).map_err(invalid)?
            || record.recorded_at().to_string() != row.get::<_, String>(2).map_err(invalid)?
        {
            return Err(Error::Invalid);
        }
        let payload = record.event().payload().as_value();
        if payload.as_object().is_none_or(|p| p.len() != 5) {
            return Err(Error::Invalid);
        }
        for (column, key) in [
            (3, "agent_type"),
            (4, "agent_instance"),
            (5, "path"),
            (6, "revision"),
            (7, "tool"),
        ] {
            if payload.get(key).and_then(serde_json::Value::as_str)
                != Some(row.get::<_, String>(column).map_err(invalid)?.as_str())
            {
                return Err(Error::Invalid);
            }
        }
        coverage
            .include(audit.verify_receipt(&record).map_err(invalid)?)
            .map_err(invalid)?;
    }
    Ok(coverage)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
