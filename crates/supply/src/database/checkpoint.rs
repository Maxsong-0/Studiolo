//! 实际只读身份视图取得的固定源；冻结只定格，放开之后在定格上拒绝未结束降级窗口并核对派生读取索引。
mod assembly;
pub use assembly::AgentCheckpointAssembly;
pub(super) use assembly::assemble;
mod reads;
mod validate;
use super::{AgentStore, SupplyError};
use std::{path::Path, time::Instant};
use studiolo_audit_ledger::{FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::{DatabaseFile, Error, FrozenSqlite, Source};
use studiolo_core::checkpoint::{CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint};

/// 已固定的 AgentStore 原库；没有写身份、迁移或初始化方法。
pub struct AgentCheckpointSource {
    source: Source,
}
/// 定格的身份快照；导出前必须校验定格并确认实际索引引用。
pub struct FrozenAgentStore {
    frozen: FrozenSqlite,
    watermark: Option<OwnerWatermark>,
    coverage: Option<ReceiptCoverage>,
}
pub(super) fn source(store: &AgentStore) -> Result<AgentCheckpointSource, SupplyError> {
    store.storage.check()?;
    let root = store
        .storage
        .database
        .parent()
        .ok_or(SupplyError::UnsafeStorage)?;
    let source = Source::bind(root, DatabaseFile::Agent).map_err(problem)?;
    store.storage.check()?;
    Ok(AgentCheckpointSource { source })
}
impl AgentCheckpointSource {
    /// 冻结窗口里只定格。逐行校验类型、实例与父链要读整张表、随使用变大（BUG-045），
    /// 放开之后由 [`FrozenAgentStore::validate`] 在定格上做，读到的仍是冻结那一刻。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenAgentStore, SupplyError> {
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        Ok(FrozenAgentStore {
            frozen,
            watermark: None,
            coverage: None,
        })
    }
}
impl FrozenAgentStore {
    /// 在定格那一刻上做完整校验：只接受已完成语义操作、没有未结束降级窗口的身份库；得出水位。
    /// 失败撤销此前的结果。
    pub fn validate(&mut self) -> Result<&OwnerWatermark, SupplyError> {
        self.watermark = None;
        let watermark = self.frozen.inspect(validate::snapshot).map_err(problem)?;
        Ok(self.watermark.insert(watermark))
    }
    /// 类型摘要为全部类型的有序摘要，不含 token 明文；实例数来自同一定格。[`Self::validate`] 通过之后才有。
    pub fn watermark(&self) -> Option<&OwnerWatermark> {
        self.watermark.as_ref()
    }
    /// 放开写锁，身份库从此可以照常写；定格的读连接留着，索引核对和导出读到的仍是冻结那一刻（BUG-045）。
    /// 之后的读取和导出受 `deadline` 约束；只能放开一次。
    pub fn release(&mut self, deadline: Instant) -> Result<(), SupplyError> {
        self.frozen.release(deadline).map_err(problem)
    }
    /// 必须传入接入方自己的审计账本，不得用人侧同名 offset 替代。
    pub fn verify_audit(&mut self, audit: &FrozenAudit) -> Result<ReceiptCoverage, SupplyError> {
        self.coverage = None;
        let coverage = self
            .frozen
            .inspect(|db| reads::verify(db, audit))
            .map_err(problem)?;
        self.coverage = Some(coverage);
        Ok(coverage)
    }
    /// 原结构、全部逻辑内容、索引原事件与持久化检查完成才出具票据。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, SupplyError> {
        self.coverage.ok_or(SupplyError::CorruptStore)?;
        let watermark = self.watermark.as_ref().ok_or(SupplyError::CorruptStore)?;
        let OwnerWatermark::AgentStore { schema_version, .. } = *watermark else {
            return Err(SupplyError::CorruptStore);
        };
        let wal = self
            .frozen
            .export(target, watermark, validate::snapshot)
            .map_err(problem)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::AgentStore,
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
/// 按封存时同一套规则在一个已打开的身份库上重算水位（类型 token 摘要、实例数、schema），只读。
/// 新根恢复用它核「放出来的库就是清单里那一份」（T-212）。
pub(super) fn restored_watermark(store: &AgentStore) -> Result<OwnerWatermark, SupplyError> {
    let connection = store.lock()?;
    validate::snapshot(&connection).map_err(problem)
}

fn problem(error: Error) -> SupplyError {
    match error {
        Error::UnsafeStorage => SupplyError::UnsafeStorage,
        Error::Invalid => SupplyError::CorruptStore,
        _ => SupplyError::StoreUnavailable,
    }
}
