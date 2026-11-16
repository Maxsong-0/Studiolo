//! 只读账本窗口：完整验链后才交付投影，内存不保留整页原始审计正文。
use super::{CreatePolicy, SqliteAuditLedger, schema, segment, unavailable, verify};
use crate::{AuditError, AuditRecord};
use rusqlite::TransactionBehavior;
use std::path::Path;

/// 倒序有界窗口；空投影页仍可能有更早记录。
pub struct AuditPage<T> {
    /// 只包含调用方投影后的字段，新到旧。
    pub records: Vec<T>,
    /// 此快照最高偏移；不是事件总数或匹配数。
    pub head_offset: u64,
    /// 本页实际检查的权威记录数。
    pub scanned: usize,
    /// 下一页的排他上界；None 表示已到起点。
    pub next_before: Option<u64>,
}

/// 不实现写者 trait，不初始化或升级账本。
pub struct AuditReadView {
    ledger: SqliteAuditLedger,
}
impl AuditReadView {
    /// 唯一开库点、只读标志、已有格式与路径身份检查。
    pub fn open_existing(directory: &Path) -> Result<Self, AuditError> {
        Ok(Self {
            ledger: SqliteAuditLedger::open_mode(directory, CreatePolicy::ExistingOnly, true)?,
        })
    }

    /// 每页至多检查256条；投影须纯计算、无副作用，整段校验失败时整个返回值作废。
    /// 段链及打开段仍按既有完整性协议核验；这里的上限指被筛选的记录窗口。
    pub fn scan_page<T>(
        &self,
        before: Option<u64>,
        mut project: impl FnMut(&AuditRecord) -> Result<Option<T>, AuditError>,
    ) -> Result<AuditPage<T>, AuditError> {
        self.ledger.storage.check()?;
        let mut session = self.ledger.lock()?;
        let tx = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        schema::validate(&tx, schema::VERSION)?;
        let anchored = verify::anchor(&tx)?;
        let upper = before.map_or(anchored.last_offset, |n| {
            n.saturating_sub(1).min(anchored.last_offset)
        });
        let lower = upper.saturating_sub(255).max(1);
        let mut records = Vec::new();
        let mut cost = anchored.cost;
        if upper > 0 {
            for index in segment::index_of(lower)?..=segment::index_of(upper)? {
                cost.merge(verify::segment_records(
                    &tx,
                    &anchored,
                    index,
                    &mut |record| {
                        if (lower..=upper).contains(&record.offset())
                            && let Some(row) = project(record)?
                        {
                            records.push(row);
                        }
                        Ok(())
                    },
                )?);
            }
        }
        self.ledger.storage.check()?;
        tx.commit().map_err(unavailable)?;
        session.cost = cost;
        records.reverse();
        Ok(AuditPage {
            records,
            head_offset: anchored.last_offset,
            scanned: if upper == 0 {
                0
            } else {
                (upper - lower + 1) as usize
            },
            next_before: (upper > 0 && lower > 1).then_some(lower),
        })
    }
}

#[cfg(test)]
#[path = "../../tests/read_view/private.rs"]
mod tests;
