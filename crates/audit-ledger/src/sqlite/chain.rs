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

use super::{schema, segment, unavailable, verify};
use crate::{AuditError, AuditSegmentSummary};
use rusqlite::Connection;
use std::sync::Arc;

/// 本连接已核过的段链前缀，以及核它时所在快照的 `data_version`。
///
/// 只放已提交的行：从已提交快照里读出并核过的行，以及本连接自己封存、提交成功之后的那一段（`push_committed_seal`）。
/// 事务里刚写下、还没提交的封存行不放，所以事务回滚不会让这里多出库里没有的段。
///
/// 锚定失败只作废 `data_version`，已放进来的段留着，当作下一次全算必须满足的前缀；其中可能有这次按段链
/// 公式核过、还没来得及锚定到 head 的行（同样读自已提交的快照），所以失败之后只会更严，不会放过什么。
#[derive(Default)]
pub(super) struct VerifiedChain {
    summaries: Arc<Vec<AuditSegmentSummary>>,
    /// `None`：还没核过，或上一次锚定没走完（失败、读不到）。下一次一律从创世全算，并核 `summaries` 这段前缀。
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
        let head = head(connection)?;
        let anchored = if known == Some(version) {
            self.extend(connection, head)?
        } else {
            self.recompute(connection, head)?
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
        let head = head(connection)?;
        let anchored = self.recompute(connection, head)?;
        self.data_version = Some(version);
        Ok(anchored)
    }

    /// 本连接刚封存、提交已经成功的那一段：放进核过的段链，此后别的连接把它退回掉，下一次操作也认得出来。
    ///
    /// 只在提交返回成功之后调用。本连接自己的提交不改变 `data_version`。写锁已随提交放掉，提交之后到这里
    /// 之间若有别的连接提交，`data_version` 会变，下一次锚定从创世全算，并要求新链以含这一段的前缀开头，
    /// 所以放进来的这一段掩盖不了别人对它的改动。段号接不上（不该发生）就作废 `data_version`，下一次全算。
    pub(super) fn push_committed_seal(&mut self, summary: AuditSegmentSummary) {
        let next = usize::try_from(summary.index()).ok() == Some(self.summaries.len());
        if self.data_version.is_some() && next {
            Arc::make_mut(&mut self.summaries).push(summary);
        } else {
            self.data_version = None;
        }
    }

    fn extend(
        &mut self,
        connection: &Connection,
        head: (u64, String),
    ) -> Result<verify::Anchored, AuditError> {
        let summaries = Arc::make_mut(&mut self.summaries);
        // 链尾那一行这次要重读；它之前的行这次不读，读到它们所在的段时再逐行比对。
        let reverified_from = u64::try_from(summaries.len().saturating_sub(1))
            .map_err(|_| AuditError::CorruptStore)?;
        let cost = segment::extend(connection, summaries)?;
        verify::settle(
            connection,
            head,
            Arc::clone(&self.summaries),
            cost,
            reverified_from,
        )
    }

    fn recompute(
        &mut self,
        connection: &Connection,
        head: (u64, String),
    ) -> Result<verify::Anchored, AuditError> {
        let mut fresh = Vec::new();
        let cost = segment::extend(connection, &mut fresh)?;
        // 段链只能往后长：本连接核过的每一段都必须原样还在，否则库被改短、改掉或整条换过。
        if !fresh.starts_with(&self.summaries) {
            return Err(AuditError::CorruptStore);
        }
        self.summaries = Arc::new(fresh);
        verify::settle(connection, head, Arc::clone(&self.summaries), cost, 0)
    }
}

/// 先读 head，再核段链：`audit_meta` 标着别的版本或 codec 时先报 `UnsupportedSchema`（要升级），
/// 与 T-220 之前的顺序一致；和 `data_version`、段链读的是同一个快照。
fn head(connection: &Connection) -> Result<(u64, String), AuditError> {
    schema::head(connection, schema::VERSION)
}

/// 在当前事务的快照里读 `data_version`；读它本身就会开始读事务，因此和随后的段链读取是同一个快照。
fn data_version(connection: &Connection) -> Result<i64, AuditError> {
    connection
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .map_err(unavailable)
}
