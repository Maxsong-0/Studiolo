//! 可信组合根的封存能力：持有实际账本实例，但不导出 append、初始化或任意 SQL。
use super::{AuditCheckpointSource, FrozenAudit, SqliteAuditLedger};
use crate::{AuditError, AuditQuiescence};
use std::{path::Path, sync::Arc, time::Instant};

/// 同一原账本的语义排空与 SQLite 封存；两项能力不能由不同路径拼接。
pub struct AuditCheckpointAccess {
    ledger: Arc<SqliteAuditLedger>,
    source: AuditCheckpointSource,
}
impl AuditCheckpointAccess {
    /// 两个实际装配共享同一原账本才可合并为一个 Audit 参与者。
    pub fn same_origin(&self, other: &Self) -> Result<bool, AuditError> {
        self.source
            .source
            .same_origin(&other.source.source)
            .map_err(super::problem)
    }
    /// 人侧复用正在服务的实际 ledger，不另开新连接伪装原 owner。
    pub fn from_ledger(ledger: Arc<SqliteAuditLedger>) -> Result<Self, AuditError> {
        let source = ledger.checkpoint_source()?;
        Ok(Self { ledger, source })
    }
    /// 独立服务由可信根绑定其既有账本；不存在或损坏即拒绝，不建空库。
    pub fn open_existing(root: &Path) -> Result<Self, AuditError> {
        Self::from_ledger(Arc::new(SqliteAuditLedger::open_existing(root)?))
    }
    /// 此刻该账本根有没有完整业务操作在做；不等待、不留租约。
    pub fn idle(&self) -> Result<bool, AuditError> {
        self.ledger.operations_idle()
    }
    /// 原截止时间内排空该账本根对应的完整业务操作。
    pub fn quiesce(&self, deadline: Instant) -> Result<AuditQuiescence, AuditError> {
        self.ledger.quiesce_operations(deadline)
    }
    /// 仅在全局语义排空后调用；固定源身份不随根路径重新解析而更新。
    pub fn freeze(&self, deadline: Instant) -> Result<FrozenAudit, AuditError> {
        self.source.freeze(deadline)
    }
}
