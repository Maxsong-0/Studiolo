//! 每类持久 ACK 显式列举，逐条有界读取；不使用最大 offset 代替原事件确认。
use super::super::{recovery, schema};
use rusqlite::Connection;
use studiolo_audit_ledger::AuditRecord;
use studiolo_checkpoint_storage::Error;

pub(super) fn require_drained(db: &Connection, version: i64) -> Result<(), Error> {
    let mut queries = vec![
        "SELECT count(*) FROM audit_outbox WHERE receipt_json IS NULL",
        "SELECT count(*) FROM auth_denials WHERE total!=delivered",
        "SELECT count(*) FROM sessions WHERE state='pending'",
        "SELECT count(*) FROM auth_retention_plan",
    ];
    if version >= 3 {
        queries.push("SELECT count(*) FROM totp_pending");
    }
    if version >= 4 {
        queries.push("SELECT count(*) FROM recovery_pending");
    }
    for query in queries {
        if db
            .query_row(query, [], |row| row.get::<_, i64>(0))
            .map_err(invalid)?
            != 0
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}
pub(super) fn visit(
    db: &Connection,
    mut visit: impl FnMut(&AuditRecord) -> Result<(), Error>,
) -> Result<(), Error> {
    let version = schema::version(db).map_err(invalid)?;
    let mut columns = vec![
        ("sessions", "grant_receipt"),
        ("audit_outbox", "receipt_json"),
        ("auth_denials", "last_receipt"),
        ("auth_retention_checkpoint", "receipt_json"),
    ];
    if version >= 3 {
        for table in ["totp_factor", "totp_pending", "totp_grants"] {
            columns.extend([(table, "intent_receipt"), (table, "outcome_receipt")]);
        }
    }
    if version >= 4 {
        for table in ["recovery_codes", "recovery_pending"] {
            columns.extend([(table, "intent_receipt"), (table, "outcome_receipt")]);
        }
    }
    for (table, column) in columns {
        // 表名和列名只来自以上封闭常量；CASE 防止损坏字段先分配无界字符串。
        let sql = format!(
            "SELECT CASE WHEN octet_length({column}) BETWEEN 1 AND 8192 THEN {column} END FROM {table} WHERE {column} IS NOT NULL"
        );
        let mut statement = db.prepare(&sql).map_err(invalid)?;
        let mut rows = statement.query([]).map_err(invalid)?;
        while let Some(row) = rows.next().map_err(invalid)? {
            let encoded: String = row.get(0).map_err(invalid)?;
            visit(&decode(&encoded)?)?;
        }
    }
    if version >= 4 {
        // 现行与替换恢复证明嵌在 recovery_control JSON；复用 schema 校验后的原事件重建。
        let id = schema::store_id(db).map_err(invalid)?;
        for (event, encoded) in recovery::restore_proofs(db, &id).map_err(invalid)? {
            let record = decode(&encoded)?;
            if record.event() != &event {
                return Err(Error::Invalid);
            }
            visit(&record)?;
        }
    }
    Ok(())
}
fn decode(encoded: &str) -> Result<AuditRecord, Error> {
    if encoded.len() > 8192 {
        return Err(Error::Invalid);
    }
    let record: AuditRecord = serde_json::from_str(encoded).map_err(invalid)?;
    record.verify_integrity().map_err(invalid)?;
    Ok(record)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
