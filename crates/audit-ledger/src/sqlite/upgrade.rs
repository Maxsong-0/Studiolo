//! 显式的 v1 → v2 分段升级：先全链验证旧账本，再一次性补齐段摘要表。
//!
//! 升级只从 T-012 的 v1 格式出发，且不改写任何既有记录或其 `record_json`：段摘要是由
//! 已验证的记录链重算出来的派生数据，因此升级后重算结果与升级前逐字节一致。
//! 对 v2、未知版本或校验不过的库一律拒绝并回滚，不隐式补表、不修补、不截短。

use super::{schema, segment, unavailable, verify};
use crate::{AuditError, AuditVerificationCost, IntegrityHash};
use rusqlite::{Connection, params};

pub(super) fn run(connection: &Connection) -> Result<AuditVerificationCost, AuditError> {
    // 只接受恰好是 v1 的库：已是 v2 或任何其它版本都在这里被明确拒绝。
    schema::validate(connection, schema::VERSION_LEGACY)?;
    schema::integrity_check(connection)?;
    let (last_offset, head_hash) = schema::head(connection, schema::VERSION_LEGACY)?;
    let (count, minimum): (i64, Option<i64>) = connection
        .query_row(
            "SELECT count(*), min(offset) FROM audit_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(unavailable)?;
    if u64::try_from(count).map_err(|_| AuditError::CorruptStore)? != last_offset
        || minimum != (last_offset > 0).then_some(1)
    {
        return Err(AuditError::CorruptStore);
    }
    connection
        .execute_batch(schema::SEGMENTS)
        .map_err(unavailable)?;
    let mut cost = AuditVerificationCost::default();
    let mut record_anchor = IntegrityHash::genesis();
    let mut segment_anchor = segment::genesis();
    let sealed = last_offset / segment::SEGMENT_RECORDS;
    for index in 0..sealed {
        let (_, last_hash, walked) = verify::walk(
            connection,
            segment::first_offset(index)?,
            segment::last_offset(index)?,
            record_anchor,
            &mut |_| Ok(()),
        )?;
        cost.merge(walked);
        let summary = segment::summarize(index, record_anchor, last_hash, segment_anchor)?;
        segment::seal(connection, &summary)?;
        record_anchor = last_hash;
        segment_anchor = summary.segment_hash();
    }
    // 尾部未写满的打开段不产生摘要；它的链尾必须正好等于旧 head。
    let (_, tail_hash, walked) = verify::walk(
        connection,
        segment::first_offset(sealed)?,
        last_offset,
        record_anchor,
        &mut |_| Ok(()),
    )?;
    cost.merge(walked);
    if tail_hash.to_hex() != head_hash {
        return Err(AuditError::CorruptStore);
    }
    let changed = connection
        .execute(
            "UPDATE audit_meta SET schema_version=?1 WHERE id=1",
            params![schema::VERSION],
        )
        .map_err(unavailable)?;
    if changed != 1 {
        return Err(AuditError::CorruptStore);
    }
    connection
        .execute_batch(&format!("PRAGMA user_version={};", schema::VERSION))
        .map_err(unavailable)?;
    // 升级后的结构必须自证为 v2，并且新段链能独立重算到同一个 head。
    schema::validate(connection, schema::VERSION)?;
    verify::anchor(connection)?;
    Ok(cost)
}
