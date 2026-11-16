//! Audit 最后冻结；全量验链后提供截止点内逐条原事件核对，不向协调器暴露连接。
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
/// 已全量验链、仍被冻结的原账本；外部只能核对既有回执和导出。
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
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenAudit, AuditError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        let watermark = frozen.inspect(snapshot).map_err(problem)?;
        Ok(FrozenAudit { frozen, watermark })
    }
}
impl FrozenAudit {
    /// 在已全量验链且写屏障仍持有的截止点内，读取一条原记录供派生索引逐字段核对。
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
    /// 已完整验链且 RESERVED 锁仍在，逐条定位只需读该条；不重复扫描整段。
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
    /// 独立主文件、完整验链、全内容摘要与 fsync 全部成立后才返回票据。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, AuditError> {
        let wal = self
            .frozen
            .export(target, &self.watermark, snapshot)
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
fn snapshot(db: &rusqlite::Connection) -> Result<OwnerWatermark, Error> {
    verify::all(db).map_err(invalid)?;
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
