//! Audit 最后冻结；冻结时只核首尾锚点并定格，提供截止点内逐条原事件核对，导出时在副本上整本验链；
//! 不向协调器暴露连接。
mod access;
pub use access::AuditCheckpointAccess;
mod coverage;
use super::{SqliteAuditLedger, query, schema, verify};
use crate::{AuditError, AuditEvent, AuditRecord, codec};
pub use coverage::{ReceiptCoverage, VerifiedReceipt};
use std::{path::Path, time::Instant};
use studiolo_checkpoint_storage::{DatabaseFile, Error, FrozenSqlite, Source};
use studiolo_core::checkpoint::{CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint};

/// 来自实际 ledger 的固定源能力；没有任意源路径或初始化入口。
pub struct AuditCheckpointSource {
    source: Source,
}
/// 定格在冻结那一刻的原账本（首尾锚点已核）；外部只能核对既有回执、放开写锁和导出。
/// 整本验链在导出时做，验不过不出票。
pub struct FrozenAudit {
    frozen: FrozenSqlite,
    watermark: OwnerWatermark,
}

impl SqliteAuditLedger {
    /// 固定本 ledger 的原数据库身份；可在全局操作屏障之前准备。
    pub fn checkpoint_source(&self) -> Result<AuditCheckpointSource, AuditError> {
        self.storage.check()?;
        let root = self
            .storage
            .database
            .parent()
            .ok_or(AuditError::UnsafeStorage)?;
        let source = Source::bind(root, DatabaseFile::Audit).map_err(problem)?;
        self.storage.check()?;
        Ok(AuditCheckpointSource { source })
    }
}
impl AuditCheckpointSource {
    /// 必须在所有上游操作与补记排空之后调用；库锁自身不能证明语义排空。
    /// 冻结窗口里只核首尾锚点（和每次读写一样的成本）；整本验链按条数涨，放到导出时在副本上做（BUG-045）。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenAudit, AuditError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        let watermark = frozen.inspect(anchored).map_err(problem)?;
        Ok(FrozenAudit { frozen, watermark })
    }
}
impl FrozenAudit {
    /// 在定格的截止点内读取一条原记录供派生索引逐字段核对（放开写锁前后读到的都一样）。
    pub fn read_record(&self, offset: u64) -> Result<AuditRecord, AuditError> {
        self.record(offset)
    }
    /// 该账本自己的截止点，不能与另一账本的 offset 混用。
    pub fn watermark(&self) -> &OwnerWatermark {
        &self.watermark
    }
    /// 逐字段核对原回执，包括记录时间、原事件、前一哈希与当前哈希。
    pub fn verify_receipt(&self, expected: &AuditRecord) -> Result<VerifiedReceipt, AuditError> {
        expected.verify_integrity()?;
        let record = self.record(expected.offset())?;
        if &record != expected {
            return Err(AuditError::CorruptStore);
        }
        Ok(VerifiedReceipt {
            offset: record.offset(),
        })
    }
    /// 对只保留原事件和 ACK 位的 owner，同样核对完整事件而非仅比较 offset。
    pub fn verify_event(&self, expected: &AuditEvent) -> Result<VerifiedReceipt, AuditError> {
        let offset = self
            .frozen
            .inspect(|db| {
                query::replay_offset(
                    db,
                    expected.event_id().as_str(),
                    codec::phase(expected.phase()),
                )
                .map_err(invalid)?
                .ok_or(Error::Invalid)
            })
            .map_err(problem)?;
        if self.record(offset)?.event() != expected {
            return Err(AuditError::CorruptStore);
        }
        Ok(VerifiedReceipt { offset })
    }
    /// 放开写锁，人侧从此可以照常追加；定格的读连接留着，回执核对和导出读到的仍是冻结那一刻。
    /// 之后的读取和导出受 `deadline` 约束。只能放开一次。
    pub fn release(&mut self, deadline: Instant) -> Result<(), AuditError> {
        self.frozen.release(deadline).map_err(problem)
    }
    /// 读定格那一刻里的一条：只读该条、核它自己的完整性，不重复扫描整段。
    /// 链有没有断由导出时在副本上整本验链兜底，验不过就不出票。
    fn record(&self, offset: u64) -> Result<AuditRecord, AuditError> {
        let OwnerWatermark::Audit { last_offset, .. } = self.watermark else {
            return Err(AuditError::CorruptStore);
        };
        if offset == 0 || offset > last_offset {
            return Err(AuditError::CorruptStore);
        }
        self.frozen.inspect(|db| {
            let (id, phase, bytes): (String, String, String) = db.query_row(
                "SELECT event_id,phase,CASE WHEN octet_length(record_json) BETWEEN 1 AND ?2 THEN record_json END FROM audit_events WHERE offset=?1",
                rusqlite::params![i64::try_from(offset).map_err(|_| Error::Invalid)?, codec::MAX_RECORD_BYTES as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).map_err(|_| Error::Invalid)?;
            let record = codec::decode(&bytes).map_err(invalid)?;
            if record.offset() != offset || record.event().event_id().as_str() != id || codec::phase(record.event().phase()) != phase
                || codec::canonical(&record).map_err(invalid)? != bytes { return Err(Error::Invalid); }
            record.verify_integrity().map_err(invalid)?;
            Ok(record)
        }).map_err(problem)
    }
    /// 独立主文件、副本整本验链（锚点必须等于冻结时那一个）、全内容摘要与 fsync 全部成立后才返回票据。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, AuditError> {
        let wal = self
            .frozen
            .export(target, &self.watermark, verified)
            .map_err(problem)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Audit,
            schema_version: schema::VERSION as u32,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: self.watermark.clone(),
        })
    }
}
/// 副本上用：整本验链之后再取锚点。
fn verified(db: &rusqlite::Connection) -> Result<OwnerWatermark, Error> {
    verify::all(db).map_err(invalid)?;
    anchored(db)
}
/// 冻结窗口里用：schema 和首尾锚点（打开段逐条、封口段核段摘要），不走读封口段里的每一条。
fn anchored(db: &rusqlite::Connection) -> Result<OwnerWatermark, Error> {
    schema::validate(db, schema::VERSION).map_err(invalid)?;
    let anchor = verify::anchor(db).map_err(invalid)?;
    Ok(OwnerWatermark::Audit {
        last_offset: anchor.last_offset,
        last_hash: anchor.last_hash.to_string(),
        agent_ledger: None,
    })
}
fn invalid(_: AuditError) -> Error {
    Error::Invalid
}
fn problem(error: Error) -> AuditError {
    match error {
        Error::UnsafeStorage => AuditError::UnsafeStorage,
        Error::Invalid => AuditError::CorruptStore,
        _ => AuditError::Unavailable,
    }
}
