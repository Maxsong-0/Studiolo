//! 许可来自本 ledger 固定根，不接受另传路径，避免与实际审计截止点错配。
use super::SqliteAuditLedger;
use crate::{AuditError, AuditQuiescence, OperationPermit};
use std::time::{Duration, Instant};

impl SqliteAuditLedger {
    pub(super) fn operation_permit(&self) -> Result<OperationPermit, AuditError> {
        self.storage
            .operation_lease(false, Instant::now() + Duration::from_secs(5))
            .map(|lease| OperationPermit {
                _lease: Some(lease),
            })
    }

    /// 试取一次完整操作的共享许可、不等待：备份冻结正占着时返回 `None`。
    ///
    /// 给不能停住线程的调用方用（异步服务在自己这边短睡重试）；同步调用方用
    /// [`crate::AuditLedger::enter_operation`]，它最多等 5 秒。
    pub fn try_enter_operation(&self) -> Result<Option<OperationPermit>, AuditError> {
        Ok(self
            .storage
            .try_operation_lease(false)?
            .map(|lease| OperationPermit {
                _lease: Some(lease),
            }))
    }

    /// 在原截止时间前排空所有使用同一审计根的完整操作。
    /// 嵌套共享许可不会被等待中的冻结者阻塞；流量过密时超时失败，不假称已冻结。
    pub fn quiesce_operations(&self, deadline: Instant) -> Result<AuditQuiescence, AuditError> {
        self.storage
            .operation_lease(true, deadline)
            .map(|lease| AuditQuiescence { _lease: lease })
    }
}
