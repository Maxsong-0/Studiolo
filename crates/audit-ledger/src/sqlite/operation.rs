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

    /// 在原截止时间前排空所有使用同一审计根的完整操作。
    /// 嵌套共享许可不会被等待中的冻结者阻塞；流量过密时超时失败，不假称已冻结。
    pub fn quiesce_operations(&self, deadline: Instant) -> Result<AuditQuiescence, AuditError> {
        self.storage
            .operation_lease(true, deadline)
            .map(|lease| AuditQuiescence { _lease: lease })
    }
}
