//! 原工作台写者的封存入口；不重开 Broker、不提供普通 mutation 能力，失败退出返还原实例。
use super::CanonicalWorkbench;
use crate::LocalError;
use std::{sync::Arc, time::Instant};
use studiolo_audit_ledger::FrozenAudit;
use studiolo_core::checkpoint::{CheckpointError, CheckpointOwner, OwnerWatermark};
use studiolo_vault_broker::{
    BrokerCapture, BrokerCheckpointAudit, BrokerCheckpointCoverage, BrokerError, FrozenBroker,
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
    /// 失败只分两类：超过固定上限（定格时文件数超过可同时打开文件数的一半）和不可用。
    pub fn verify_audit(
        &mut self,
        audit: &FrozenAudit,
    ) -> Result<BrokerCheckpointCoverage, CheckpointError> {
        self.source().map_err(|_| broker_failure(None))?;
        self.broker
            .as_mut()
            .ok_or_else(|| broker_failure(None))?
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
            .map_err(|error| broker_failure(Some(&error)))
    }
    /// 交出经原账本核验过的定格（两棵树的文件句柄，BUG-045）；之后丢掉本窗口就把写者还回工作台，
    /// 拷贝在窗口之外从定格做（`studiolo_vault_broker::BrokerCapture::export`）。原根身份在交出前后复验。
    pub fn take_capture(&mut self) -> Result<BrokerCapture, LocalError> {
        self.source()?;
        let capture = self
            .broker
            .as_mut()
            .ok_or(LocalError::Unavailable)?
            .take_capture()
            .map_err(|_| LocalError::Unavailable)?;
        self.source()?;
        Ok(capture)
    }
}
/// 写者在窗口里失败的类别。定格要同时开着两棵树的每个文件，超过上限时单独报出来，
/// 现场才知道该去调打开文件数，而不是和别的写者故障混成一句「不可用」（BUG-045）。
fn broker_failure(error: Option<&BrokerError>) -> CheckpointError {
    let owner = CheckpointOwner::Broker;
    match error {
        Some(BrokerError::WriterLimitExceeded) => CheckpointError::LimitExceeded { owner },
        _ => CheckpointError::OwnerUnavailable { owner },
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

#[cfg(test)]
mod tests {
    use super::{BrokerError, CheckpointError, CheckpointOwner, broker_failure};

    #[test]
    fn a_writer_over_the_open_file_limit_is_reported_apart_from_other_failures() {
        let owner = CheckpointOwner::Broker;
        assert_eq!(
            broker_failure(Some(&BrokerError::WriterLimitExceeded)),
            CheckpointError::LimitExceeded { owner }
        );
        for other in [Some(&BrokerError::CaptureChanged), None] {
            assert_eq!(
                broker_failure(other),
                CheckpointError::OwnerUnavailable { owner }
            );
        }
    }
}
