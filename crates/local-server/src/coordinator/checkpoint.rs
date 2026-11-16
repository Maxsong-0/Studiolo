//! 原工作台写者的封存入口；不重开 Broker、不提供普通 mutation 能力，失败退出返还原实例。
use super::CanonicalWorkbench;
use crate::LocalError;
use std::{path::Path, sync::Arc, time::Instant};
use studiolo_audit_ledger::FrozenAudit;
use studiolo_core::checkpoint::{OwnerWatermark, SealedOwnerCheckpoint};
use studiolo_vault_broker::{
    BrokerCheckpointAudit, BrokerCheckpointCoverage, BrokerError, FrozenBroker,
};

/// 保留原工作台与唯一写者的窗口；全局操作屏障由上层协调器持有。
pub struct FrozenWorkbench {
    workbench: Arc<CanonicalWorkbench>,
    broker: Option<FrozenBroker>,
}
impl CanonicalWorkbench {
    /// 只移动正在服务的那个 Broker；忙、关停、根漂移、未决或未交付审计均拒绝。
    pub fn freeze_checkpoint(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<FrozenWorkbench, LocalError> {
        if self
            .core
            .control
            .try_lock()
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        self.core
            .roots
            .revalidate()
            .map_err(|_| LocalError::Unavailable)?;
        let mut owner = self
            .core
            .owner
            .try_lock()
            .map_err(|_| LocalError::Unavailable)?;
        owner
            .as_ref()
            .ok_or(LocalError::Unavailable)?
            .validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )
            .map_err(|_| LocalError::Unavailable)?;
        let broker = owner
            .take()
            .ok_or(LocalError::Unavailable)?
            .freeze_checkpoint(deadline);
        // 先释放槽锁再检查：任何失败均经下面的 RAII 返还，避免 Drop 自锁。
        drop(owner);
        let frozen = FrozenWorkbench {
            workbench: self.clone(),
            broker: Some(broker),
        };
        frozen.watermark()?;
        Ok(frozen)
    }
}
impl FrozenWorkbench {
    fn source(&self) -> Result<&FrozenBroker, LocalError> {
        self.workbench
            .core
            .roots
            .revalidate()
            .map_err(|_| LocalError::Unavailable)?;
        self.broker.as_ref().ok_or(LocalError::Unavailable)
    }
    /// 实际业务提交水位与无未决证据。
    pub fn watermark(&self) -> Result<OwnerWatermark, LocalError> {
        self.source()?
            .watermark()
            .map_err(|_| LocalError::Unavailable)
    }
    /// 复用线上唯一事件映射，再按持久 offset/hash 与完整原事件核对真实人侧账本。
    pub fn verify_audit(
        &mut self,
        audit: &FrozenAudit,
    ) -> Result<BrokerCheckpointCoverage, LocalError> {
        self.source()?;
        self.broker
            .as_mut()
            .ok_or(LocalError::Unavailable)?
            .verify_audit(|candidate, offset, hash| {
                let expected = match candidate {
                    BrokerCheckpointAudit::Content(value) => crate::editing::audit::event(value)
                        .map_err(|_| BrokerError::CorruptWriterState)?,
                    BrokerCheckpointAudit::FileOperation(value) => {
                        crate::file_operation_audit::event(value)
                            .map_err(|_| BrokerError::CorruptWriterState)?
                    }
                    BrokerCheckpointAudit::Maintenance(value) => {
                        crate::maintenance::audit::event(value)
                            .map_err(|_| BrokerError::CorruptWriterState)?
                    }
                };
                let original = audit
                    .read_record(offset)
                    .map_err(|_| BrokerError::CorruptWriterState)?;
                if original.event() != &expected || original.record_hash().to_hex() != hash {
                    return Err(BrokerError::CorruptWriterState);
                }
                audit
                    .verify_receipt(&original)
                    .map(|_| ())
                    .map_err(|_| BrokerError::CorruptWriterState)
            })
            .map_err(|_| LocalError::Unavailable)
    }
    /// 只导出经原账本核验后的独立副本；原根身份在复制前后复验。
    pub fn export(&self, vault: &Path, writer: &Path) -> Result<SealedOwnerCheckpoint, LocalError> {
        let sealed = self
            .source()?
            .export(vault, writer)
            .map_err(|_| LocalError::Unavailable)?;
        self.source()?;
        Ok(sealed)
    }
}
impl Drop for FrozenWorkbench {
    fn drop(&mut self) {
        let mut owner = self
            .workbench
            .core
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner.is_none() {
            *owner = self.broker.take().map(FrozenBroker::thaw);
        }
    }
}
