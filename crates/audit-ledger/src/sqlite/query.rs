//! 把 `AuditFilter` 翻译成走索引的候选行查询，再按段验证候选所在的整段后取结果。
//!
//! 只有落在真实列上的条件能下推：`offset` 走 rowid 主键，`event_id`（可带 `phase`）走
//! `UNIQUE(event_id, phase)` 自动索引。task/attempt/tool/action 只存在于规范 JSON 正文里，
//! 没有索引，因此这类过滤仍要走遍候选区间——契约文档把这条代价写明，不假称全部条件都有索引。

use super::{segment, unavailable, verify};
use crate::{AuditError, AuditFilter, AuditRecord, AuditVerificationCost, codec};
use rusqlite::{Connection, types::Value};

struct Candidates {
    sql: String,
    parameters: Vec<Value>,
}

/// 仅测试可见：返回真实执行的候选 SQL 及 SQLite 给出的执行计划，用来核对「确实走索引」。
///
/// 计划由被测连接上的 `EXPLAIN QUERY PLAN` 直接产出，并且用与生产同一条 `plan` 结果，
/// 不是另写一条 SQL 来自证。
#[cfg(test)]
pub(super) fn explain(
    connection: &Connection,
    filter: &AuditFilter,
) -> Result<(String, String), AuditError> {
    let candidates = plan(filter);
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {}", candidates.sql))
        .map_err(unavailable)?;
    let mut rows = statement
        .query(rusqlite::params_from_iter(candidates.parameters))
        .map_err(unavailable)?;
    let mut detail = String::new();
    while let Some(row) = rows.next().map_err(unavailable)? {
        detail.push_str(&row.get::<_, String>(3).map_err(unavailable)?);
        detail.push(';');
    }
    Ok((candidates.sql, detail))
}

fn plan(filter: &AuditFilter) -> Candidates {
    let mut parameters = Vec::new();
    let mut sql = String::from("SELECT offset FROM audit_events WHERE ");
    // event_id 是唯一索引的首列，必须排在最前面，SQLite 才会选中该索引而不是整表扫描。
    if let Some(event_id) = filter.event_id() {
        parameters.push(Value::Text(event_id.as_str().to_owned()));
        sql.push_str("event_id = ? AND ");
        if let Some(phase) = filter.phase() {
            parameters.push(Value::Text(codec::phase(phase).to_owned()));
            sql.push_str("phase = ? AND ");
        }
    } else if let Some(phase) = filter.phase() {
        parameters.push(Value::Text(codec::phase(phase).to_owned()));
        sql.push_str("phase = ? AND ");
    }
    // 超出 i64 的 after_offset 不可能命中任何记录；退化为空结果而不是报错。
    let after = i64::try_from(filter.offset_exclusive().unwrap_or(0)).unwrap_or(i64::MAX);
    parameters.push(Value::Integer(after));
    sql.push_str("offset > ? ORDER BY offset");
    if let Some(limit) = pushable_limit(filter) {
        parameters.push(Value::Integer(limit));
        sql.push_str(" LIMIT ?");
    }
    Candidates { sql, parameters }
}

/// 只有全部条件都能下推时才下推 limit；否则候选集必须完整，由 Rust 侧再判定。
fn pushable_limit(filter: &AuditFilter) -> Option<i64> {
    if filter.task_id().is_some()
        || filter.attempt_id().is_some()
        || filter.tool().is_some()
        || filter.action().is_some()
    {
        return None;
    }
    filter.limit().and_then(|limit| i64::try_from(limit).ok())
}

/// 按候选行定位需要验证的段，逐段验证后在已验证记录上应用完整过滤。
///
/// 候选按 offset 升序到达，因此每段至多验证一次，结果保持追加顺序。返回的每条记录
/// 都由所在段的记录链重算过，并经段摘要锚定到 head；不返回未经验证的记录。
pub(super) fn run(
    connection: &Connection,
    anchored: &verify::Anchored,
    filter: &AuditFilter,
) -> Result<(Vec<AuditRecord>, AuditVerificationCost), AuditError> {
    let mut records = Vec::new();
    let mut cost = AuditVerificationCost::default();
    let limit = filter.limit().unwrap_or(usize::MAX);
    if limit == 0 {
        return Ok((records, cost));
    }
    let candidates = plan(filter);
    let mut statement = connection.prepare(&candidates.sql).map_err(unavailable)?;
    let mut rows = statement
        .query(rusqlite::params_from_iter(candidates.parameters))
        .map_err(unavailable)?;
    let mut visited: Option<u64> = None;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let offset = u64::try_from(row.get::<_, i64>(0).map_err(unavailable)?)
            .map_err(|_| AuditError::CorruptStore)?;
        let index = segment::index_of(offset)?;
        if visited == Some(index) {
            continue;
        }
        visited = Some(index);
        cost.merge(verify::segment_records(
            connection,
            anchored,
            index,
            &mut |record| {
                if records.len() < limit && filter.matches(record) {
                    records.push(record.clone());
                }
                Ok(())
            },
        )?);
        if records.len() >= limit {
            break;
        }
    }
    Ok((records, cost))
}

/// 用幂等键定位既有记录的 offset；走 `UNIQUE(event_id, phase)` 索引，不随账本增长。
pub(super) fn replay_offset(
    connection: &Connection,
    event_id: &str,
    phase: &str,
) -> Result<Option<u64>, AuditError> {
    let offset: Option<i64> = connection
        .query_row(
            "SELECT offset FROM audit_events WHERE event_id = ?1 AND phase = ?2",
            rusqlite::params![event_id, phase],
            |row| row.get(0),
        )
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(unavailable(other)),
        })?;
    offset
        .map(|value| u64::try_from(value).map_err(|_| AuditError::CorruptStore))
        .transpose()
}
