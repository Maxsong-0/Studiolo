//! 在同一 SQLite 快照内按段验链：段链核到链尾，记录链只走实际读到的段，两者用摘要衔接。
//!
//! 每次操作都做三件事：把段链核到链尾（本连接核过、别的连接没动过的前缀只重读链尾那一行，
//! 见 `chain.rs`；否则从创世重算）、走读当前打开段、核对 head。因此段被删除、段号或段边界
//! 被改写、摘要被替换、尾部记录被增删改，只要经别的 SQLite 连接提交，都当场失败关闭。
//! **已封存段的内部改写只有在走读该段时才发现**（`all`、`segment`，或读到该段的查询）；读到
//! 某段时连该段的摘要行一起重读比对。这是分段的代价，已在契约文档中逐条写明，不在此处假装覆盖。

use super::{chain, schema, segment, unavailable};
use crate::{
    AuditError, AuditRecord, AuditSegmentSummary, AuditVerificationCost, IntegrityHash, codec,
};
use rusqlite::{Connection, params};
use std::sync::Arc;

/// 已把 head 锚定到段链与打开段的结果。
pub(super) struct Anchored {
    /// 核过的全部封存段摘要；与本连接的段链缓存共用一份，不按操作复制。
    pub(super) summaries: Arc<Vec<AuditSegmentSummary>>,
    /// 段号小于它的摘要行这次快照里没重读（沿用本连接早先核过的那份）；读到这些段时要逐行比对。
    pub(super) reverified_from: u64,
    pub(super) last_offset: u64,
    pub(super) last_hash: IntegrityHash,
    pub(super) cost: AuditVerificationCost,
}

impl Anchored {
    /// 返回当前打开段的段号，即已封存段数。
    pub(super) fn open_index(&self) -> Result<u64, AuditError> {
        u64::try_from(self.summaries.len()).map_err(|_| AuditError::CorruptStore)
    }
}

/// 取账本首、尾偏移的两句 SQL，必须各写一句：单独一个 `min` 或 `max` 走一次 rowid 主键查找；两个聚合写进
/// 同一句，SQLite 就不走这条捷径，改成把唯一索引整个扫一遍，70 万条时每次约 72 毫秒，而这里是每次读写
/// 账本都要过的（BUG-042）。执行计划用例逐句核对它们不扫整本账本。
pub(super) const BOUNDS: [&str; 2] = [
    "SELECT min(offset) FROM audit_events",
    "SELECT max(offset) FROM audit_events",
];

/// 重算段链、走读打开段并核对 head；成本为 O(段数 + 打开段记录数)，与账本总量无关。
///
/// 上一句的「与账本总量无关」只指与记录总字节无关，段数仍随总量涨。这是没有段链缓存的一次性连接
/// （封存核对、升级）用的入口，从创世全算；常规操作走 `chain::VerifiedChain::anchor`，别的连接没提交过时只核链尾。
pub(super) fn anchor(connection: &Connection) -> Result<Anchored, AuditError> {
    chain::VerifiedChain::default().anchor_from_genesis(connection)
}

/// 段链已核到 `summaries` 之后，读 head、核打开段位置与首尾边界、走读打开段；成本为 O(打开段记录数)。
pub(super) fn settle(
    connection: &Connection,
    summaries: Arc<Vec<AuditSegmentSummary>>,
    mut cost: AuditVerificationCost,
    reverified_from: u64,
) -> Result<Anchored, AuditError> {
    let (last_offset, head_hash) = schema::head(connection, schema::VERSION)?;
    let sealed = u64::try_from(summaries.len()).map_err(|_| AuditError::CorruptStore)?;
    let open_first = segment::first_offset(sealed)?;
    // 打开段必须紧接最后一段且尚未写满；否则该段本应已封存，或 head 落在段链之前。
    if last_offset.checked_add(1).ok_or(AuditError::CorruptStore)? < open_first
        || last_offset >= segment::last_offset(sealed)?
    {
        return Err(AuditError::CorruptStore);
    }
    // 记录行必须正好落在 [1, last_offset]：首、尾各查一次，都走 rowid 主键（见 `BOUNDS`），
    // 因此整条账本的首尾边界每次操作都核对，只有中间段的缺行要靠走读该段发现。
    let bound = |sql: &str| -> Result<Option<i64>, AuditError> {
        connection
            .query_row(sql, [], |row| row.get(0))
            .map_err(unavailable)
    };
    let (minimum, maximum) = (bound(BOUNDS[0])?, bound(BOUNDS[1])?);
    let observed = match maximum {
        None => 0,
        Some(value) => u64::try_from(value).map_err(|_| AuditError::CorruptStore)?,
    };
    if observed != last_offset || minimum != (last_offset > 0).then_some(1) {
        return Err(AuditError::CorruptStore);
    }
    let previous = summaries
        .last()
        .map_or_else(IntegrityHash::genesis, AuditSegmentSummary::last_hash);
    let (_, last_hash, walked) = walk(connection, open_first, last_offset, previous, &mut |_| {
        Ok(())
    })?;
    cost.merge(walked);
    if last_hash.to_hex() != head_hash {
        return Err(AuditError::CorruptStore);
    }
    Ok(Anchored {
        summaries,
        reverified_from,
        last_offset,
        last_hash,
        cost,
    })
}

/// 走读一整段并把其中每条已验证记录交给 visitor；段尾哈希必须与摘要或 head 一致。
///
/// 这一段的摘要行这次快照里没重读过（沿用本连接早先核过的那份）时，先重读它逐字段比对：读到哪段，
/// 那段的记录和摘要行都当场核过。
pub(super) fn segment_records(
    connection: &Connection,
    anchored: &Anchored,
    index: u64,
    visitor: &mut dyn FnMut(&AuditRecord) -> Result<(), AuditError>,
) -> Result<AuditVerificationCost, AuditError> {
    let first = segment::first_offset(index)?;
    let open = anchored.open_index()?;
    let mut cost = AuditVerificationCost::default();
    let (last, expected_hash) = if index < open {
        let summary = summary_at(anchored, index)?;
        if index < anchored.reverified_from {
            cost.merge(segment::recheck(connection, summary)?);
        }
        (summary.last_offset(), summary.last_hash())
    } else if index == open {
        (anchored.last_offset, anchored.last_hash)
    } else {
        return Err(AuditError::CorruptStore);
    };
    let previous = match index.checked_sub(1) {
        None => IntegrityHash::genesis(),
        Some(earlier) => summary_at(anchored, earlier)?.last_hash(),
    };
    let (_, actual, walked) = walk(connection, first, last, previous, visitor)?;
    if actual != expected_hash {
        return Err(AuditError::CorruptStore);
    }
    cost.merge(walked);
    Ok(cost)
}

/// 取出某个 offset 的记录，并连带验证它所在的整段；不返回未经验证的记录。
pub(super) fn record_at(
    connection: &Connection,
    anchored: &Anchored,
    offset: u64,
) -> Result<(AuditRecord, AuditVerificationCost), AuditError> {
    if offset == 0 || offset > anchored.last_offset {
        return Err(AuditError::CorruptStore);
    }
    let mut found = None;
    let cost = segment_records(
        connection,
        anchored,
        segment::index_of(offset)?,
        &mut |record| {
            if record.offset() == offset {
                found = Some(record.clone());
            }
            Ok(())
        },
    )?;
    Ok((found.ok_or(AuditError::CorruptStore)?, cost))
}

/// 全量验证：物理结构检查、行数与 offset 边界、段链、每一段的记录链与打开段。
///
/// 这是本 Adapter 对「账本完整」的完整定义，成本与账本总量成正比，因此是显式入口，
/// 不放进 append/query 的热路径。没有段链缓存的一次性连接（封存核对）用它。
pub(super) fn all(connection: &Connection) -> Result<AuditVerificationCost, AuditError> {
    all_with(connection, &mut chain::VerifiedChain::default())
}

/// 同 [`all`]，段链从创世全算后与本连接核过的前缀比对，并刷新它。
pub(super) fn all_with(
    connection: &Connection,
    chain: &mut chain::VerifiedChain,
) -> Result<AuditVerificationCost, AuditError> {
    schema::validate(connection, schema::VERSION)?;
    schema::integrity_check(connection)?;
    let anchored = chain.anchor_from_genesis(connection)?;
    // 全量验证才付得起的整表统计：连续 offset 下行数必须正好等于 head，任何缺行都要发现。
    let count: i64 = connection
        .query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0))
        .map_err(unavailable)?;
    if u64::try_from(count).map_err(|_| AuditError::CorruptStore)? != anchored.last_offset {
        return Err(AuditError::CorruptStore);
    }
    let mut cost = anchored.cost;
    for index in 0..anchored.open_index()? {
        cost.merge(segment_records(connection, &anchored, index, &mut |_| {
            Ok(())
        })?);
    }
    Ok(cost)
}

fn summary_at(anchored: &Anchored, index: u64) -> Result<&AuditSegmentSummary, AuditError> {
    anchored
        .summaries
        .get(usize::try_from(index).map_err(|_| AuditError::CorruptStore)?)
        .ok_or(AuditError::CorruptStore)
}

/// 按 offset 升序走读 `[from, to]`，逐条重算记录链；空区间直接返回锚点。
pub(super) fn walk(
    connection: &Connection,
    from: u64,
    to: u64,
    anchor: IntegrityHash,
    visitor: &mut dyn FnMut(&AuditRecord) -> Result<(), AuditError>,
) -> Result<(u64, IntegrityHash, AuditVerificationCost), AuditError> {
    let mut cost = AuditVerificationCost::default();
    if from > to {
        return Ok((0, anchor, cost));
    }
    let mut hash = anchor;
    let mut current = from.checked_sub(1).ok_or(AuditError::CorruptStore)?;
    // octet_length 直接检查列的字节长度；CASE 只投影有界字段，不能先把超长正文取回。
    // 这是应用读取边界，不声称约束 SQLite 的页缓存、schema 解析等全部内存。
    let mut statement = connection
        .prepare(
            "SELECT offset,
            CASE WHEN octet_length(event_id) BETWEEN 1 AND ?1 THEN event_id END,
            CASE WHEN octet_length(phase) BETWEEN 6 AND 7 THEN phase END,
            CASE WHEN octet_length(record_json) BETWEEN 1 AND ?1 THEN record_json END
         FROM audit_events WHERE offset BETWEEN ?2 AND ?3 ORDER BY offset",
        )
        .map_err(unavailable)?;
    let mut rows = statement
        .query(params![
            codec::MAX_RECORD_BYTES as i64,
            i64::try_from(from).map_err(|_| AuditError::CorruptStore)?,
            i64::try_from(to).map_err(|_| AuditError::CorruptStore)?
        ])
        .map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        cost.add_record();
        let offset: i64 = row.get(0).map_err(unavailable)?;
        let event_id = bounded_text(row, 1)?;
        let phase = bounded_text(row, 2)?;
        let encoded = bounded_text(row, 3)?;
        let record = codec::decode(&encoded).map_err(|_| AuditError::CorruptStore)?;
        let expected = current.checked_add(1).ok_or(AuditError::CorruptStore)?;
        if u64::try_from(offset).ok() != Some(expected)
            || record.offset() != expected
            || record.previous_hash() != hash
            || record.event().event_id().as_str() != event_id
            || codec::phase(record.event().phase()) != phase
        {
            return Err(AuditError::CorruptStore);
        }
        let rebuilt = codec::build(
            record.offset(),
            record.recorded_at(),
            hash,
            record.event().clone(),
        )
        .map_err(|_| AuditError::CorruptStore)?;
        if rebuilt != record
            || codec::canonical(&record).map_err(|_| AuditError::CorruptStore)? != encoded
        {
            return Err(AuditError::CorruptStore);
        }
        visitor(&record)?;
        current = record.offset();
        hash = record.record_hash();
    }
    if current != to {
        return Err(AuditError::CorruptStore);
    }
    Ok((
        to.checked_sub(from)
            .and_then(|value| value.checked_add(1))
            .ok_or(AuditError::CorruptStore)?,
        hash,
        cost,
    ))
}

fn bounded_text(row: &rusqlite::Row<'_>, index: usize) -> Result<String, AuditError> {
    row.get::<_, Option<String>>(index)
        .map_err(unavailable)?
        .ok_or(AuditError::CorruptStore)
}
