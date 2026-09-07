//! 在同一 SQLite 快照内流式验链；过滤或重放不允许绕过其余记录的验证。

use super::{schema, unavailable};
use crate::{AuditError, AuditEvent, AuditFilter, AuditRecord, IntegrityHash, codec};
use rusqlite::{Connection, params};

pub(super) struct Verified {
    pub(super) offset: u64,
    pub(super) hash: IntegrityHash,
    pub(super) replay: Option<AuditRecord>,
    pub(super) records: Vec<AuditRecord>,
}

pub(super) fn scan(
    connection: &Connection,
    filter: Option<&AuditFilter>,
    replay: Option<&AuditEvent>,
) -> Result<Verified, AuditError> {
    schema::validate(connection)?;
    let head = schema::head(connection)?;
    let mut verified = Verified {
        offset: 0,
        hash: IntegrityHash::genesis(),
        replay: None,
        records: Vec::new(),
    };
    // octet_length 直接检查列的字节长度；CASE 只投影有界字段，不能先把超长正文取回。
    // 这是应用读取边界，不声称约束 SQLite 的页缓存、schema 解析等全部内存。
    let mut statement = connection
        .prepare(
            "SELECT offset,
            CASE WHEN octet_length(event_id) BETWEEN 1 AND ?1 THEN event_id END,
            CASE WHEN octet_length(phase) BETWEEN 6 AND 7 THEN phase END,
            CASE WHEN octet_length(record_json) BETWEEN 1 AND ?1 THEN record_json END
         FROM audit_events ORDER BY offset",
        )
        .map_err(unavailable)?;
    let mut rows = statement
        .query(params![codec::MAX_RECORD_BYTES as i64])
        .map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let offset: i64 = row.get(0).map_err(unavailable)?;
        let event_id = bounded_text(row, 1)?;
        let phase = bounded_text(row, 2)?;
        let encoded = bounded_text(row, 3)?;
        let record = codec::decode(&encoded).map_err(|_| AuditError::CorruptStore)?;
        let expected_offset = verified
            .offset
            .checked_add(1)
            .ok_or(AuditError::CorruptStore)?;
        if u64::try_from(offset).ok() != Some(expected_offset)
            || record.offset() != expected_offset
            || record.previous_hash() != verified.hash
            || record.event().event_id().as_str() != event_id
            || codec::phase(record.event().phase()) != phase
        {
            return Err(AuditError::CorruptStore);
        }
        let expected = codec::build(
            record.offset(),
            record.recorded_at(),
            verified.hash,
            record.event().clone(),
        )
        .map_err(|_| AuditError::CorruptStore)?;
        if expected != record
            || codec::canonical(&record).map_err(|_| AuditError::CorruptStore)? != encoded
        {
            return Err(AuditError::CorruptStore);
        }
        verified.offset = record.offset();
        verified.hash = record.record_hash();
        if replay.is_some_and(|event| {
            event.event_id() == record.event().event_id() && event.phase() == record.event().phase()
        }) {
            verified.replay = Some(record.clone());
        }
        if let Some(filter) = filter
            && verified.records.len() < filter.limit().unwrap_or(usize::MAX)
            && filter.matches(&record)
        {
            verified.records.push(record);
        }
    }
    if head != (verified.offset, verified.hash.to_hex()) {
        return Err(AuditError::CorruptStore);
    }
    Ok(verified)
}

fn bounded_text(row: &rusqlite::Row<'_>, index: usize) -> Result<String, AuditError> {
    row.get::<_, Option<String>>(index)
        .map_err(unavailable)?
        .ok_or(AuditError::CorruptStore)
}
