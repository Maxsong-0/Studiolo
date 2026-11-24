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

/// 已在服务启动时固定 inode 的控制存储。克隆的是同一份固定身份，不重新开库。
#[derive(Clone)]
pub struct ControlCheckpointSource {
    source: Source,
    vault: VaultUuid,
}
/// 同时阻止所有 SQLite 写者的窗口；定格校验过、原审计确认过才能签出副本。
pub struct FrozenControl {
    frozen: FrozenSqlite,
    vault: VaultUuid,
    watermark: Option<OwnerWatermark>,
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
    /// 复核固定的目录和库还是启动时那一份；不碰数据库连接，所以不需要黑板的引擎锁（BUG-055）。
    pub fn check(&self) -> Result<(), DbError> {
        self.source.check().map_err(problem)
    }
    /// 冻结窗口里只定格，不在锁内执行外部副作用。排空与完整校验要读整张表、随使用变大（BUG-045），
    /// 放开之后由 [`FrozenControl::validate`] 在定格上做，读到的仍是冻结那一刻。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenControl, DbError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        Ok(FrozenControl {
            frozen,
            vault: self.vault,
            watermark: None,
            coverage: None,
        })
    }
}
impl FrozenControl {
    /// 在定格那一刻上做完整校验：投影、终态和运行时审计 outbox 均已排空，结构、业务引用与 Vault 绑定都对；
    /// 得出 Control 水位。失败撤销此前的结果。
    pub fn validate(&mut self) -> Result<&OwnerWatermark, DbError> {
        self.watermark = None;
        let watermark = self
            .frozen
            .inspect(|db| validate::snapshot(db, self.vault))
            .map_err(problem)?;
        Ok(self.watermark.insert(watermark))
    }
    /// Control 自身的投影 outbox 水位与 Vault 绑定；[`Self::validate`] 通过之后才有。
    pub fn watermark(&self) -> Option<&OwnerWatermark> {
        self.watermark.as_ref()
    }
    /// 放开写锁，控制库从此可以照常写；定格的读连接留着，回执核对和导出读到的仍是冻结那一刻（BUG-045）。
    /// 之后的读取和导出受 `deadline` 约束；只能放开一次。
    pub fn release(&mut self, deadline: Instant) -> Result<(), DbError> {
        self.frozen.release(deadline).map_err(problem)
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
    /// 定格校验过、原审计确认过，副本完整核验及 fsync 后返回封存票据。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, DbError> {
        self.coverage.ok_or_else(|| problem(Error::Invalid))?;
        let watermark = self
            .watermark
            .as_ref()
            .ok_or_else(|| problem(Error::Invalid))?;
        let OwnerWatermark::Control { schema_version, .. } = *watermark else {
            return Err(problem(Error::Invalid));
        };
        let wal = self
            .frozen
            .export(target, watermark, |db| validate::snapshot(db, self.vault))
            .map_err(problem)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Control,
            schema_version,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: watermark.clone(),
        })
    }
}
impl ControlDb {
    /// 新根恢复专用（T-212）：只打开封存里放出来的固定 `control.sqlite3`。不建库（文件不在就失败）、不迁移、
    /// 不切持久 journal mode、不写 Vault 绑定：只做连接级设置，再核 schema 已经是当前版本、Vault 绑定已经在
    /// 库里且等于 `vault_uuid`。水位要核的是放出来的原样，迁移的补建、补行和 WAL 切换都不能先于核对发生。
    pub fn open_existing(root: &Path, vault_uuid: VaultUuid) -> Result<Self, DbError> {
        let connection = rusqlite::Connection::open_with_flags(
            root.join("control.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let db = Self::configured(connection, vault_uuid)?;
        if db.schema_version()? != crate::migration::SCHEMA_VERSION
            || db.bound_vault_uuid()? != vault_uuid
        {
            return Err(problem(Error::Invalid));
        }
        Ok(db)
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
