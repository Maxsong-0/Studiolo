//! Auth 冻结保留真实主人绑定；所有 ACK 必须在最后冻结的原审计账本内逐条确认。
mod receipts;
use super::{SessionConfig, SessionError, SessionManager, schema};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Instant};
use studiolo_audit_ledger::{FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::{Error, FrozenSqlite, Source};
use studiolo_core::checkpoint::{CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint};

/// 来自实际 SessionManager 的固定源能力，不创建或迁移认证库。
pub struct AuthCheckpointSource {
    source: Source,
    config: SessionConfig,
}
/// 定格的 Auth 快照；定格校验与完整 ACK 覆盖都成立前不允许导出。
pub struct FrozenAuth {
    frozen: FrozenSqlite,
    config: String,
    /// 主人绑定的摘要，冻结时由配置算出；校验和导出都按它核。
    owner: String,
    watermark: Option<OwnerWatermark>,
    coverage: Option<ReceiptCoverage>,
}
impl SessionManager {
    /// 在全局操作暂停之前固定正在服务的认证库身份。
    pub fn checkpoint_source(&self) -> Result<AuthCheckpointSource, SessionError> {
        Ok(AuthCheckpointSource {
            source: self.database.checkpoint_source()?,
            config: self.config.clone(),
        })
    }
}
impl AuthCheckpointSource {
    /// 冻结窗口里只定格。排空与回执的完整校验要读整张表、随使用变大（BUG-045），
    /// 放开之后由 [`FrozenAuth::validate`] 在定格上做，读到的仍是冻结那一刻。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenAuth, SessionError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        let config = self.config.encoded()?;
        let owner =
            serde_json::to_vec(&self.config.owner).map_err(|_| SessionError::Configuration)?;
        let owner = super::secrets::hex(&Sha256::digest(owner));
        Ok(FrozenAuth {
            frozen,
            config,
            owner,
            watermark: None,
            coverage: None,
        })
    }
}
impl FrozenAuth {
    /// 在定格那一刻上做完整校验：只接受已完成补记和业务回写的库，待处理记录不能冒充已排空；
    /// 得出本库水位。失败撤销此前的结果。
    pub fn validate(&mut self) -> Result<&OwnerWatermark, SessionError> {
        self.watermark = None;
        let watermark = self
            .frozen
            .inspect(|db| snapshot(db, &self.config, &self.owner))
            .map_err(problem)?;
        Ok(self.watermark.insert(watermark))
    }
    /// 本库独立的业务水位，不等同于审计尾部；[`Self::validate`] 通过之后才有。
    pub fn watermark(&self) -> Option<&OwnerWatermark> {
        self.watermark.as_ref()
    }
    /// 放开写锁，登录库从此可以照常写；定格的读连接留着，回执核对和导出读到的仍是冻结那一刻（BUG-045）。
    /// 之后的读取和导出受 `deadline` 约束；只能放开一次。
    pub fn release(&mut self, deadline: Instant) -> Result<(), SessionError> {
        self.frozen.release(deadline).map_err(problem)
    }
    /// 扫描所有持久回执，包括恢复控制 JSON 内的原证明；一次失败撤销此前确认。
    pub fn verify_audit(&mut self, audit: &FrozenAudit) -> Result<ReceiptCoverage, SessionError> {
        self.coverage = None;
        let coverage = self
            .frozen
            .inspect(|db| {
                let mut coverage = ReceiptCoverage::default();
                receipts::visit(db, |record| {
                    coverage
                        .include(audit.verify_receipt(record).map_err(|_| Error::Invalid)?)
                        .map_err(|_| Error::Invalid)
                })?;
                Ok(coverage)
            })
            .map_err(problem)?;
        self.coverage = Some(coverage);
        Ok(coverage)
    }
    /// 只有完整结构、排空和原账本核对通过，才能签出独立可重开的副本。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, SessionError> {
        self.coverage.ok_or(SessionError::AuditUnavailable)?;
        let watermark = self.watermark.as_ref().ok_or(SessionError::CorruptStore)?;
        let OwnerWatermark::Auth { schema_version, .. } = watermark else {
            return Err(SessionError::CorruptStore);
        };
        let wal = self
            .frozen
            .export(target, watermark, |db| {
                snapshot(db, &self.config, &self.owner)
            })
            .map_err(problem)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Auth,
            schema_version: *schema_version,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: watermark.clone(),
        })
    }
}
fn snapshot(db: &rusqlite::Connection, config: &str, owner: &str) -> Result<OwnerWatermark, Error> {
    schema::validate(db, config).map_err(|_| Error::Invalid)?;
    let version = schema::version(db).map_err(|_| Error::Invalid)?;
    if !(2..=4).contains(&version) {
        return Err(Error::Invalid);
    }
    receipts::require_drained(db, version)?;
    let mut maximum = 0;
    receipts::visit(db, |receipt| {
        maximum = maximum.max(receipt.offset());
        Ok(())
    })?;
    Ok(OwnerWatermark::Auth {
        schema_version: version as u32,
        owner_binding_digest: owner.to_owned(),
        audit_anchor_offset: maximum,
    })
}
pub(super) fn problem(error: Error) -> SessionError {
    match error {
        Error::UnsafeStorage => SessionError::UnsafeStorage,
        Error::Invalid => SessionError::CorruptStore,
        _ => SessionError::StorageUnavailable,
    }
}
