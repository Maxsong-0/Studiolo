//! 固定边界的段划分、段摘要哈希与段链验证。
//!
//! 段链是记录链之上的第二层链：每段封存时写下一行摘要，摘要里带上一段摘要的哈希。
//! 段链从创世重算即可发现整段被删除、段号被改写、段边界被挪动或摘要被替换；
//! 在本连接已核过的前缀之后续核（`extend`）只读链尾那一行和新增的行，什么时候能这样做由 `chain.rs` 判定。
//! 摘要里的 `last_hash` 只有在实际走读该段记录时才与内容对上，这一点由 `verify` 之外
//! 的记录级验证负责，本模块不声称已证明任一段的内容。

use super::unavailable;
use crate::{AuditError, AuditSegmentSummary, AuditVerificationCost, IntegrityHash, codec};
use rusqlite::Connection;

/// 每段固定记录条数；段边界只由 offset 决定，与时间、大小或调用方无关。
pub(crate) const SEGMENT_RECORDS: u64 = 128;

/// 返回 offset 所属段号；offset 从 1 起，0 不属于任何段。
pub(crate) fn index_of(offset: u64) -> Result<u64, AuditError> {
    offset
        .checked_sub(1)
        .map(|value| value / SEGMENT_RECORDS)
        .ok_or(AuditError::CorruptStore)
}

/// 返回该段第一条记录的 offset。
pub(crate) fn first_offset(index: u64) -> Result<u64, AuditError> {
    index
        .checked_mul(SEGMENT_RECORDS)
        .and_then(|value| value.checked_add(1))
        .ok_or(AuditError::CorruptStore)
}

/// 返回该段最后一条记录的 offset；只有写满这条 offset 段才封存。
pub(crate) fn last_offset(index: u64) -> Result<u64, AuditError> {
    index
        .checked_add(1)
        .and_then(|value| value.checked_mul(SEGMENT_RECORDS))
        .ok_or(AuditError::CorruptStore)
}

/// 段链创世哈希；与记录链创世哈希分属不同 domain，不可互换。
pub(crate) fn genesis() -> IntegrityHash {
    IntegrityHash::sha256(b"studiolo-audit-segment-genesis-v1")
}

/// 计算段摘要哈希：domain、上一段摘要哈希的原始字节，以及段号与两端边界的规范编码。
pub(crate) fn hash(
    index: u64,
    previous_hash: IntegrityHash,
    last_hash: IntegrityHash,
    previous_segment_hash: IntegrityHash,
) -> Result<IntegrityHash, AuditError> {
    let encoded = codec::canonical(&(
        index,
        first_offset(index)?,
        last_offset(index)?,
        previous_hash,
        last_hash,
    ))?;
    let mut bytes = b"studiolo-audit-segment-v1\0".to_vec();
    bytes.extend_from_slice(&previous_segment_hash.bytes());
    bytes.extend_from_slice(encoded.as_bytes());
    Ok(IntegrityHash::sha256(&bytes))
}

/// 由段内两端哈希组装一段摘要；调用方负责保证两端哈希来自已验证的记录链。
pub(crate) fn summarize(
    index: u64,
    previous_hash: IntegrityHash,
    last_hash: IntegrityHash,
    previous_segment_hash: IntegrityHash,
) -> Result<AuditSegmentSummary, AuditError> {
    Ok(AuditSegmentSummary::from_parts(
        index,
        first_offset(index)?,
        last_offset(index)?,
        previous_hash,
        last_hash,
        previous_segment_hash,
        hash(index, previous_hash, last_hash, previous_segment_hash)?,
    ))
}

/// 读段摘要行的列；摘要为定长十六进制，先约束列字节长度，再取回最多两段 64 字节文本。
const STORED: &str = "SELECT segment_index, last_offset,
            CASE WHEN octet_length(last_hash) = 64 THEN last_hash END,
            CASE WHEN octet_length(segment_hash) = 64 THEN segment_hash END
         FROM audit_segments";

/// 在已核过的段链前缀之后接着核，把核过的新段追加进 `summaries`。
///
/// 前缀为空就是从创世重算整条段链。前缀不空时先重读前缀最后一行、要求它与当初核过的那份逐字段相同，
/// 再按段链公式核它之后的每一行；前缀里更早的行这次不读。调用方必须能证明它们自核过以后没被别的连接改过
/// （见 `chain.rs`），否则就该传空前缀全算。成本与「1 + 新增段数」成正比，前缀为空时与段数成正比；
/// 每段一行定长摘要，与记录总字节无关。段号必须连续，段边界必须落在固定倍数上，任一处不符即失败关闭，
/// 不跳过、不修补；失败时 `summaries` 里可能已多出几行核过的新段，它们仍是合法前缀。
pub(crate) fn extend(
    connection: &Connection,
    summaries: &mut Vec<AuditSegmentSummary>,
) -> Result<AuditVerificationCost, AuditError> {
    let mut cost = AuditVerificationCost::default();
    let verified = u64::try_from(summaries.len()).map_err(|_| AuditError::CorruptStore)?;
    // 全算时从最小的行读起（含被篡改成负数的段号，读到即拒绝）；续核时从前缀最后一行读起。
    let (first, from) = match verified.checked_sub(1) {
        None => (0, i64::MIN),
        Some(tip) => (
            tip,
            i64::try_from(tip).map_err(|_| AuditError::CorruptStore)?,
        ),
    };
    let mut statement = connection
        .prepare(&format!(
            "{STORED} WHERE segment_index >= ?1 ORDER BY segment_index"
        ))
        .map_err(unavailable)?;
    let mut rows = statement.query([from]).map_err(unavailable)?;
    let mut expected_index = first;
    while let Some(row) = rows.next().map_err(unavailable)? {
        cost.add_summary();
        let stored = Stored::read(row)?;
        if stored.index != expected_index {
            return Err(AuditError::CorruptStore);
        }
        if expected_index < verified {
            let known = usize::try_from(expected_index).map_err(|_| AuditError::CorruptStore)?;
            stored.must_equal(&summaries[known])?;
        } else {
            let (previous_hash, previous_segment_hash) = summaries.last().map_or_else(
                || (IntegrityHash::genesis(), genesis()),
                |previous| (previous.last_hash(), previous.segment_hash()),
            );
            let summary = summarize(
                expected_index,
                previous_hash,
                stored.last_hash,
                previous_segment_hash,
            )?;
            stored.must_equal(&summary)?;
            summaries.push(summary);
        }
        expected_index = expected_index
            .checked_add(1)
            .ok_or(AuditError::CorruptStore)?;
    }
    // 前缀最后一行必须还在：被删了，上面一行都读不到。
    if expected_index < verified {
        return Err(AuditError::CorruptStore);
    }
    Ok(cost)
}

/// 重读一行已核过的段摘要，必须与当初核过的那份逐字段相同；读到某段时用，不重算段链。
pub(crate) fn recheck(
    connection: &Connection,
    known: &AuditSegmentSummary,
) -> Result<AuditVerificationCost, AuditError> {
    let mut cost = AuditVerificationCost::default();
    let mut statement = connection
        .prepare(&format!("{STORED} WHERE segment_index = ?1"))
        .map_err(unavailable)?;
    let mut rows = statement
        .query([i64::try_from(known.index()).map_err(|_| AuditError::CorruptStore)?])
        .map_err(unavailable)?;
    let row = rows
        .next()
        .map_err(unavailable)?
        .ok_or(AuditError::CorruptStore)?;
    cost.add_summary();
    let stored = Stored::read(row)?;
    if stored.index != known.index() {
        return Err(AuditError::CorruptStore);
    }
    stored.must_equal(known)?;
    Ok(cost)
}

/// 库里一行段摘要的原样；段边界和 `previous_hash` 不存盘，由段号与前一行重算。
struct Stored {
    index: u64,
    last_offset: u64,
    last_hash: IntegrityHash,
    segment_hash: String,
}

impl Stored {
    fn read(row: &rusqlite::Row<'_>) -> Result<Self, AuditError> {
        Ok(Self {
            index: u64::try_from(row.get::<_, i64>(0).map_err(unavailable)?)
                .map_err(|_| AuditError::CorruptStore)?,
            last_offset: u64::try_from(row.get::<_, i64>(1).map_err(unavailable)?)
                .map_err(|_| AuditError::CorruptStore)?,
            last_hash: hex(row, 2)?,
            segment_hash: fixed_text(row, 3)?,
        })
    }

    /// 存盘的三列必须正是这份摘要的；`segment_hash` 覆盖段号、边界、两端哈希与上一段，相等即整行相同。
    fn must_equal(&self, summary: &AuditSegmentSummary) -> Result<(), AuditError> {
        if self.last_offset != summary.last_offset()
            || self.last_hash != summary.last_hash()
            || self.segment_hash != summary.segment_hash().to_hex()
        {
            return Err(AuditError::CorruptStore);
        }
        Ok(())
    }
}

/// 写入一段封存摘要；只在段内最后一条记录落库的同一事务里调用。
pub(crate) fn seal(
    connection: &Connection,
    summary: &AuditSegmentSummary,
) -> Result<(), AuditError> {
    let inserted = connection
        .execute(
            "INSERT INTO audit_segments(segment_index,last_offset,last_hash,segment_hash)
             VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                i64::try_from(summary.index()).map_err(|_| AuditError::Unavailable)?,
                i64::try_from(summary.last_offset()).map_err(|_| AuditError::Unavailable)?,
                summary.last_hash().to_hex(),
                summary.segment_hash().to_hex(),
            ],
        )
        .map_err(unavailable)?;
    if inserted != 1 {
        return Err(AuditError::CorruptStore);
    }
    Ok(())
}

fn fixed_text(row: &rusqlite::Row<'_>, index: usize) -> Result<String, AuditError> {
    row.get::<_, Option<String>>(index)
        .map_err(unavailable)?
        .ok_or(AuditError::CorruptStore)
}

fn hex(row: &rusqlite::Row<'_>, index: usize) -> Result<IntegrityHash, AuditError> {
    IntegrityHash::from_hex(&fixed_text(row, index)?).ok_or(AuditError::CorruptStore)
}
