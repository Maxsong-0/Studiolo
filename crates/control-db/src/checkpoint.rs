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
impl ControlDb {
    /// 新根恢复专用（T-212）：只打开封存里放出来的固定 `control.sqlite3`。不建库（文件不在就失败）、不迁移：
    /// schema 必须已经是当前版本、Vault 绑定必须已经在库里且等于 `vault_uuid`，都核过才走正常的连接配置。
    pub fn open_existing(root: &Path, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        let connection = rusqlite::Connection::open_with_flags(
            root.join("control.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version),0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        let stored: String = connection.query_row(
            "SELECT vault_uuid FROM vault_binding WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if version != crate::migration::SCHEMA_VERSION
            || crate::types::parse_vault_uuid(&stored)? != vault_uuid
        {
            return Err(problem(Error::Invalid));
        }
        Self::from_connection(connection, vault_uuid)
    }

    /// 按封存时同一套规则在这个库上重算 Control 水位（schema、Vault 绑定、各 outbox 排空、没有提交中的
    /// attempt、完整性检查），只读。新根恢复用它核「放出来的库就是清单里那一份」。
    pub fn restored_watermark(&self) -> Result<OwnerWatermark, DbError> {
        validate::snapshot(&self.connection, self.vault_uuid).map_err(problem)
    }

    /// 还活着的 attempt 数，只读。新根恢复撤销之后拿着租约的必须是 0。
    pub fn live_attempts(&self) -> Result<LiveAttempts, DbError> {
        let (leased, cancel_requested): (i64, i64) = self.connection.query_row(
            "SELECT count(*) FILTER (WHERE phase IN ('claimed','running')),
                    count(*) FILTER (WHERE phase='cancel_requested')
             FROM attempts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(LiveAttempts {
            leased: u64::try_from(leased).map_err(|_| problem(Error::Invalid))?,
            cancel_requested: u64::try_from(cancel_requested)
                .map_err(|_| problem(Error::Invalid))?,
        })
    }
}

/// 还活着的 attempt 分两类：拿着租约的（已认领、运行中）和已经收回工具能力、只等结束的（取消中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveAttempts {
    /// 已认领或运行中。
    pub leased: u64,
    /// 取消中：工具能力已经收回，只等子任务终结或取消宽限到期。
    pub cancel_requested: u64,
}

pub(super) fn problem(error: Error) -> DbError {
    DbError::Corrupt(error.to_string())
}
