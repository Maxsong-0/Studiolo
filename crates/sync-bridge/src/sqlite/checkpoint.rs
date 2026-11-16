//! Bridge 的实际库与原租约一起冻结；持久传输门禁由真实装配提供并在导出前后复验。
mod snapshot;
use super::{BridgeError, BridgeState, model::State, storage::HeldLease};
use crate::Binding;
use std::{path::Path, sync::Arc, time::Instant};
use studiolo_audit_ledger::{FrozenAudit, ReceiptCoverage};
use studiolo_checkpoint_storage::{DatabaseFile, Error, FrozenSqlite, Source};
use studiolo_core::checkpoint::{CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint};

/// 可信装配持有的真实传输暂停能力；不能用布尔值或最近退出时间替代。
/// 实现须检查固定 binding、持久关闭的 spawn 门禁、无活动受控进程和原租约，且遵守截止时间。
pub trait BridgeCheckpointGate: Send + Sync {
    /// 每次返回前复验能力仍有效；新轮次、根/租约变动或到期均拒绝。
    fn verify(&self, binding: &Binding, deadline: Instant) -> Result<(), BridgeError>;
}
/// 固定原库与原 bridge.lock 租约；即使原 owner 被丢弃，另一个进程仍不能接管。
pub struct BridgeCheckpointSource {
    source: Source,
    binding: Binding,
    lease: Arc<HeldLease>,
}
/// 完整已验证状态、单库写屏障与真实传输门禁共同存活的窗口。
pub struct FrozenBridge {
    frozen: FrozenSqlite,
    state: State,
    watermark: OwnerWatermark,
    gate: Arc<dyn BridgeCheckpointGate>,
    deadline: Instant,
    coverage: Option<ReceiptCoverage>,
    _lease: Arc<HeldLease>,
}
impl BridgeState {
    /// 仅从正在服务的已绑定 owner 取能力；不重新打开第二个 BridgeState。
    pub fn checkpoint_source(&self) -> Result<BridgeCheckpointSource, BridgeError> {
        let binding = self.read()?.binding.clone();
        let root = self
            .storage
            .database
            .parent()
            .ok_or(BridgeError::UnsafeStorage)?;
        let source = Source::bind(root, DatabaseFile::Bridge).map_err(problem)?;
        self.storage.check()?;
        Ok(BridgeCheckpointSource {
            source,
            binding,
            lease: self.storage.retain_lease(),
        })
    }
}
impl BridgeCheckpointSource {
    /// 不以单库锁冒充传输静止；还有未闭合计划或未补记时失败。
    pub fn freeze(
        &self,
        gate: Arc<dyn BridgeCheckpointGate>,
        deadline: Instant,
    ) -> Result<FrozenBridge, BridgeError> {
        gate.verify(&self.binding, deadline)?;
        let frozen = self.source.freeze(deadline).map_err(problem)?;
        let state = frozen
            .inspect(|db| snapshot::load(db, &self.binding))
            .map_err(problem)?;
        let watermark = snapshot::watermark(&state).map_err(problem)?;
        gate.verify(&self.binding, deadline)?;
        Ok(FrozenBridge {
            frozen,
            state,
            watermark,
            gate,
            deadline,
            coverage: None,
            _lease: self.lease.clone(),
        })
    }
}
impl FrozenBridge {
    /// 附属文件导出后复验原传输门禁和库 binding；不重新创建或覆盖任何目标。
    pub fn verify_source(&self) -> Result<(), BridgeError> {
        self.gate.verify(&self.state.binding, self.deadline)?;
        self.frozen
            .inspect(|db| snapshot::load(db, &self.state.binding).map(|_| ()))
            .map_err(problem)
    }

    /// 共同基线内容摘要和 canonical 观察序号；并非索引 generation 或同步退出时间。
    pub fn watermark(&self) -> &OwnerWatermark {
        &self.watermark
    }
    /// 核对全部仍持久保留的动作、结果、汇总与熔断原回执。
    pub fn verify_audit(&mut self, audit: &FrozenAudit) -> Result<ReceiptCoverage, BridgeError> {
        self.coverage = None;
        self.gate.verify(&self.state.binding, self.deadline)?;
        let coverage = self
            .frozen
            .inspect(|_| snapshot::verify_audit(&self.state, audit))
            .map_err(problem)?;
        self.gate.verify(&self.state.binding, self.deadline)?;
        self.coverage = Some(coverage);
        Ok(coverage)
    }
    /// 库副本通过全量核验后再次核对真实门禁；附属传输/平面文件仍由它们的 owner 封存。
    pub fn export(&self, target: &Path) -> Result<SealedOwnerCheckpoint, BridgeError> {
        self.coverage.ok_or(BridgeError::AuditUnavailable)?;
        self.gate.verify(&self.state.binding, self.deadline)?;
        let wal = self
            .frozen
            .export(target, &self.watermark, |db| {
                snapshot::watermark(&snapshot::load(db, &self.state.binding)?)
            })
            .map_err(problem)?;
        self.gate.verify(&self.state.binding, self.deadline)?;
        Ok(SealedOwnerCheckpoint {
            owner: CheckpointOwner::Bridge,
            schema_version: 1,
            wal,
            target_synced: true,
            parent_directory_synced: true,
            reopen_verified: true,
            source_binding_stable: true,
            watermark: self.watermark.clone(),
        })
    }
}
fn problem(error: Error) -> BridgeError {
    match error {
        Error::UnsafeStorage => BridgeError::UnsafeStorage,
        Error::Invalid => BridgeError::CorruptStore,
        _ => BridgeError::Unavailable,
    }
}
