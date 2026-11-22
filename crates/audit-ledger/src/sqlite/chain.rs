//! 本连接核过的段链：从创世重算一次后留在内存，之后只要没有别的连接提交过，每次操作只核链尾和新封存的段。
//!
//! 判据是 SQLite 的 `PRAGMA data_version`：这期间只要有**别的连接**向该库提交过改动，同一条连接两次读到的值就一定
//! 不同；本连接自己的提交不改变它（SQLite 文档的承诺；别的原因也可能让它变，那只是多全算一次）。值没变，库里就只多了本连接自己写的东西（新记录、新封存行、
//! head），已核过的段摘要行一行没动：重读链尾那一行、核它之后新封存的行就够了，成本与段数无关。
//!
//! 值变了（另一个实例追加、有人经别的连接改库），不知道动了哪里，就从创世全算整条段链，并要求新链以
//! 本连接核过的那段为前缀：别的连接把已封存的段改短、改掉，或整条换成另一条自身自洽的链，都当场失败关闭。
//!
//! 不经 SQLite、直接改文件字节的改动不在这条判据之内（以前每次全算也可能命中页缓存而读不到它）；这类改动由读到
//! 该段时的摘要行比对（`verify::segment_records`）、`verify_all`、`segments()` 和重新打开时的全算发现，见契约第 8 节。

use super::{segment, unavailable, verify};
use crate::{AuditError, AuditSegmentSummary};
use rusqlite::Connection;
use std::sync::Arc;

/// 本连接已核过的段链前缀，以及核它时所在快照的 `data_version`。
///
/// 只放从已提交快照里读出并核过的行，本连接自己在事务里刚写下、还没提交的封存行不放：下一次操作从库里读它、
/// 照段链公式核过才进来，所以事务回滚不会让这里多出库里没有的段。
#[derive(Default)]
pub(super) struct VerifiedChain {
    summaries: Arc<Vec<AuditSegmentSummary>>,
    /// `None`：还没核过，或上一次锚定没走完（失败、读不到）。下一次一律从创世全算。
    data_version: Option<i64>,
}

impl VerifiedChain {
    /// 常规操作（追加、查询、读窗口、按段续验）的锚定：别的连接没提交过就只核链尾，提交过就从创世全算。
    pub(super) fn anchor(
        &mut self,
        connection: &Connection,
    ) -> Result<verify::Anchored, AuditError> {
        // 先作废再干活：从读 `data_version` 起任何一步失败，下一次都一律全算；只有整次锚定走完才重新记下版本。
        let known = self.data_version.take();
        let version = data_version(connection)?;
        let anchored = if known == Some(version) {
            self.extend(connection)?
        } else {
            self.recompute(connection)?
        };
        self.data_version = Some(version);
        Ok(anchored)
    }

    /// 显式入口（打开、`verify_all`、`segments()`、一次性连接）：不看 `data_version`，从创世全算。
    pub(super) fn anchor_from_genesis(
        &mut self,
        connection: &Connection,
    ) -> Result<verify::Anchored, AuditError> {
        self.data_version = None;
        let version = data_version(connection)?;
        let anchored = self.recompute(connection)?;
        self.data_version = Some(version);
        Ok(anchored)
    }

    fn extend(&mut self, connection: &Connection) -> Result<verify::Anchored, AuditError> {
        let summaries = Arc::make_mut(&mut self.summaries);
        // 链尾那一行这次要重读；它之前的行这次不读，读到它们所在的段时再逐行比对。
        let reverified_from = u64::try_from(summaries.len().saturating_sub(1))
            .map_err(|_| AuditError::CorruptStore)?;
        let cost = segment::extend(connection, summaries)?;
        verify::settle(
            connection,
            Arc::clone(&self.summaries),
            cost,
            reverified_from,
        )
    }

    fn recompute(&mut self, connection: &Connection) -> Result<verify::Anchored, AuditError> {
        let mut fresh = Vec::new();
        let cost = segment::extend(connection, &mut fresh)?;
        // 段链只能往后长：本连接核过的每一段都必须原样还在，否则库被改短、改掉或整条换过。
        if !fresh.starts_with(&self.summaries) {
            return Err(AuditError::CorruptStore);
        }
        self.summaries = Arc::new(fresh);
        verify::settle(connection, Arc::clone(&self.summaries), cost, 0)
    }
}

/// 在当前事务的快照里读 `data_version`；读它本身就会开始读事务，因此和随后的段链读取是同一个快照。
fn data_version(connection: &Connection) -> Result<i64, AuditError> {
    connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(unavailable)
}
