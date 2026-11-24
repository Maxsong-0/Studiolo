//! 原工作台写者的封存入口；不重开 Broker、不提供普通 mutation 能力，失败退出返还原实例。
use super::CanonicalWorkbench;
use crate::{LocalError, bounded_lock};
use std::{sync::Arc, time::Instant};
use studiolo_audit_ledger::FrozenAudit;
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner, OwnerWatermark, SealedOwnerCheckpoint,
};
use studiolo_vault_broker::{
    BrokerCapture, BrokerCheckpointAudit, BrokerCheckpointCoverage, BrokerError, ExportedBroker,
    FrozenBroker,
};

/// 保留原工作台与唯一写者的窗口；全局操作屏障由上层协调器持有。
pub struct FrozenWorkbench {
    workbench: Arc<CanonicalWorkbench>,
    broker: Option<FrozenBroker>,
}
impl CanonicalWorkbench {
    /// 只移动正在服务的那个 Broker；关停、根漂移、未决或未交付审计均拒绝。
    /// 排空完整操作之后仍可能有不取许可的短暂持有者（网页读一篇笔记、监视回调、就绪检查），
    /// 所以两把锁都最多等 [`bounded_lock::BRIEF`]（BUG-055）；一直不放的算忙、照旧拒绝。
    pub fn freeze_checkpoint(
        self: &Arc<Self>,
        deadline: Instant,
    ) -> Result<FrozenWorkbench, LocalError> {
        if bounded_lock::lock_until(&self.core.control, bounded_lock::brief(deadline))
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        self.core
            .roots
            .revalidate()
            .map_err(|_| LocalError::Unavailable)?;
        let mut owner = bounded_lock::lock_until(&self.core.owner, bounded_lock::brief(deadline))
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
    /// 窗口里定格写者的两棵树（水位、定格、复核，不读回执，BUG-050）；逐条核回执在放开之后、
    /// 导出的副本上做（[`verify_exported`]）。失败的类别见 `broker_failure`。
    pub fn capture(&mut self) -> Result<(), CheckpointError> {
        self.source().map_err(|_| broker_failure(None))?;
        self.broker
            .as_mut()
            .ok_or_else(|| broker_failure(None))?
            .capture()
            .map_err(|error| broker_failure(Some(&error)))
    }
    /// 交出定格（两棵树的文件句柄，BUG-045）；之后丢掉本窗口就把写者还回工作台，
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
/// 放开之后、在导出的写者副本上逐条核回执（BUG-050）：复用线上唯一事件映射，再按持久 offset/hash 与完整
/// 原事件核对真实人侧账本（冻结那一刻的定格）。核完才有写者的票据。
pub(crate) fn verify_exported(
    exported: ExportedBroker,
    audit: &FrozenAudit,
    deadline: Instant,
) -> Result<(SealedOwnerCheckpoint, BrokerCheckpointCoverage), CheckpointError> {
    exported
        .verify(deadline, |candidate, offset, hash| {
            receipt_matches(audit, candidate, offset, hash)
        })
        .map_err(|error| broker_failure(Some(&error)))
}
/// 回执指的那条账本记录就是这个事件，偏移、完整事件、哈希一样不差。
fn receipt_matches(
    audit: &FrozenAudit,
    candidate: BrokerCheckpointAudit<'_>,
    offset: u64,
    hash: &str,
) -> Result<(), BrokerError> {
    let expected = match candidate {
        BrokerCheckpointAudit::Content(value) => {
            crate::editing::audit::event(value).map_err(|_| BrokerError::CorruptWriterState)?
        }
        BrokerCheckpointAudit::FileOperation(value) => crate::file_operation_audit::event(value)
            .map_err(|_| BrokerError::CorruptWriterState)?,
        BrokerCheckpointAudit::Maintenance(value) => {
            crate::maintenance::audit::event(value).map_err(|_| BrokerError::CorruptWriterState)?
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
}
/// 写者定格（窗口里）、从定格导出、在副本上核回执（放开之后）失败的类别，三处共用（BUG-045、BUG-050）。
/// 超过固定上限（打开文件数、目录深度、节点数、字节数、重放条数、目标可用空间）和定格之后被改过各自单独报，
/// 现场才知道该去调上限、查盘，还是查谁原地改了文件，而不是和别的写者故障混成一句「不可用」。
pub(crate) fn broker_failure(error: Option<&BrokerError>) -> CheckpointError {
    let owner = CheckpointOwner::Broker;
    match error {
        Some(BrokerError::WriterLimitExceeded) => CheckpointError::LimitExceeded { owner },
        Some(BrokerError::CaptureChanged) => CheckpointError::SourceChanged { owner },
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
    fn a_writer_over_a_limit_or_changed_after_capture_is_reported_apart_from_other_failures() {
        let owner = CheckpointOwner::Broker;
        assert_eq!(
            broker_failure(Some(&BrokerError::WriterLimitExceeded)),
            CheckpointError::LimitExceeded { owner }
        );
        assert_eq!(
            broker_failure(Some(&BrokerError::CaptureChanged)),
            CheckpointError::SourceChanged { owner }
        );
        for other in [Some(&BrokerError::IncompleteCapture), None] {
            assert_eq!(
                broker_failure(other),
                CheckpointError::OwnerUnavailable { owner }
            );
        }
    }
}
