//! 固定 Control owner 的封存能力；不把连接交给协调器，也不借重新打开执行迁移。
mod receipts;
mod validate;
use crate::{ControlDb, DbError};
use std::{path::Path, time::Instant};
use studiolo_audit_ledger::{FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::{Error, FrozenSqlite, Source};
use studiolo_core::{
    VaultUuid,
    checkpoint::{CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint},
};

/// 已在服务启动时固定 inode 的控制存储。
pub struct ControlCheckpointSource {
    source: Source,
    vault: VaultUuid,
}
/// 同时阻止所有 SQLite 写者的窗口；原审计确认前不能签出副本。
pub struct FrozenControl {
    frozen: FrozenSqlite,
    watermark: OwnerWatermark,
    coverage: Option<ReceiptCoverage>,
}
impl ControlDb {
    /// 内存库与未走私有固定路径入口的旧适配器不具备备份能力。
    pub fn checkpoint_source(&self) -> Result<ControlCheckpointSource, DbError> {
        let source = self
            .checkpoint_source
            .as_ref()
            .ok_or_else(|| problem(Error::UnsafeStorage))?;
        source.check().map_err(problem)?;
        Ok(ControlCheckpointSource {
            source: source.clone(),
            vault: self.vault_uuid,
        })
    }
}
impl ControlCheckpointSource {
    /// 要求投影、终态和运行时审计 outbox 均已排空；不在锁内执行外部副作用。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenControl, DbError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        let watermark = frozen
            .inspect(|db| validate::snapshot(db, self.vault))
            .map_err(problem)?;
        Ok(FrozenControl {
            frozen,
            watermark,
            coverage: None,
        })
    }
}
impl FrozenControl {
    /// Control 自身的投影 outbox 水位与 Vault 绑定。
    pub fn watermark(&self) -> &OwnerWatermark {
        &self.watermark
    }
    /// 对所有已投递运行时与终态结果核对完整原事件，而非只比较序号。
    pub fn verify_audit(&mut self, audit: &FrozenAudit) -> Result<ReceiptCoverage, DbError> {
        self.coverage = None;
        let coverage = self
            .frozen
            .inspect(|db| receipts::verify(db, audit))
            .map_err(problem)?;
        self.coverage = Some(coverage);
        Ok(coverage)
    }
    /// 完整核验及 fsync 后返回封存票据；仍保持写屏障直到整个窗口释放。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, DbError> {
        self.coverage.ok_or_else(|| problem(Error::Invalid))?;
        let OwnerWatermark::Control {
            vault_uuid,
            schema_version,
            ..
        } = self.watermark
        else {
            return Err(problem(Error::Invalid));
        };
        let wal = self
            .frozen
            .export(target, &self.watermark, |db| {
                validate::snapshot(db, vault_uuid)
            })
            .map_err(problem)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Control,
            schema_version,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: self.watermark.clone(),
        })
    }
}
pub(super) fn problem(error: Error) -> DbError {
    DbError::Corrupt(error.to_string())
}
