//! 分段摘要与验证规模的只读视图。
//!
//! 这些类型描述「某次验证实际做了什么」，本身不是完整性证明：它们只能由
//! Adapter 在完成对应验证后构造，crate 外无法伪造一个「已验证」的结论。

use crate::IntegrityHash;

/// 一段已封存审计记录的摘要。
///
/// 段号从 0 起连续编号，每段固定 [`AUDIT_SEGMENT_RECORDS`](crate::AUDIT_SEGMENT_RECORDS) 条
/// 记录。`previous_hash` 是段内第一条记录的前驱哈希，`last_hash` 是段内最后一条记录的
/// `record_hash`；`segment_hash` 把段号、段边界、两端哈希连同上一段的 `segment_hash`
/// 一起摘要，从而把所有段串成一条可从创世重算的段链。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditSegmentSummary {
    index: u64,
    first_offset: u64,
    last_offset: u64,
    previous_hash: IntegrityHash,
    last_hash: IntegrityHash,
    previous_segment_hash: IntegrityHash,
    segment_hash: IntegrityHash,
}

impl AuditSegmentSummary {
    pub(crate) fn from_parts(
        index: u64,
        first_offset: u64,
        last_offset: u64,
        previous_hash: IntegrityHash,
        last_hash: IntegrityHash,
        previous_segment_hash: IntegrityHash,
        segment_hash: IntegrityHash,
    ) -> Self {
        Self {
            index,
            first_offset,
            last_offset,
            previous_hash,
            last_hash,
            previous_segment_hash,
            segment_hash,
        }
    }

    /// 返回从 0 起的段号。
    #[must_use]
    pub fn index(&self) -> u64 {
        self.index
    }

    /// 返回段内第一条记录的 offset。
    #[must_use]
    pub fn first_offset(&self) -> u64 {
        self.first_offset
    }

    /// 返回段内最后一条记录的 offset。
    #[must_use]
    pub fn last_offset(&self) -> u64 {
        self.last_offset
    }

    /// 返回段内第一条记录的前驱哈希：上一段的 `last_hash`，首段为记录链创世哈希。
    #[must_use]
    pub fn previous_hash(&self) -> IntegrityHash {
        self.previous_hash
    }

    /// 返回段内最后一条记录的 `record_hash`。
    #[must_use]
    pub fn last_hash(&self) -> IntegrityHash {
        self.last_hash
    }

    /// 返回上一段的 `segment_hash`，首段为段链创世哈希。
    #[must_use]
    pub fn previous_segment_hash(&self) -> IntegrityHash {
        self.previous_segment_hash
    }

    /// 返回本段在段链中的摘要哈希。
    #[must_use]
    pub fn segment_hash(&self) -> IntegrityHash {
        self.segment_hash
    }
}

/// 一次验证实际读入并重算的规模。
///
/// `summaries` 是读入并重算的段摘要行数，`records` 是解码并重算哈希的审计记录条数。
/// 它用于核对「分段后常规操作的成本不再等于账本总量」这一断言，本身不是完整性结论：
/// 数值小只说明这次验证读得少，不说明未读部分正确。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuditVerificationCost {
    summaries: u64,
    records: u64,
}

impl AuditVerificationCost {
    pub(crate) fn add_summary(&mut self) {
        self.summaries = self.summaries.saturating_add(1);
    }

    pub(crate) fn add_record(&mut self) {
        self.records = self.records.saturating_add(1);
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.summaries = self.summaries.saturating_add(other.summaries);
        self.records = self.records.saturating_add(other.records);
    }

    /// 返回本次验证读入并重算的段摘要行数。
    #[must_use]
    pub fn summaries(&self) -> u64 {
        self.summaries
    }

    /// 返回本次验证解码并重算哈希的审计记录条数。
    #[must_use]
    pub fn records(&self) -> u64 {
        self.records
    }
}
