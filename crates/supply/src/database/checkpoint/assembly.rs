//! 身份库和固定 audit 子目录成对取证；调用方不能把人侧 ledger 当作接入方链传入。
use super::super::AgentStore;
use super::{AgentCheckpointSource, FrozenAgentStore, SupplyError, source};
use std::time::Instant;
use studiolo_audit_ledger::{AuditCheckpointAccess, AuditQuiescence, FrozenAudit};

/// 只含冻结和排空能力；不提供身份变更、审计追加或初始化入口。
pub struct AgentCheckpointAssembly {
    source: AgentCheckpointSource,
    audit: AuditCheckpointAccess,
}
pub(in crate::database) fn assemble(
    store: &AgentStore,
) -> Result<AgentCheckpointAssembly, SupplyError> {
    let source = source(store)?;
    let root = store
        .storage
        .database
        .parent()
        .ok_or(SupplyError::UnsafeStorage)?;
    let audit = AuditCheckpointAccess::open_existing(&root.join("audit"))
        .map_err(|_| SupplyError::StoreUnavailable)?;
    store.storage.check()?;
    Ok(AgentCheckpointAssembly { source, audit })
}
impl AgentCheckpointAssembly {
    /// 身份库及其附属账本都必须仍是同一原对象；同路径替换不算同一来源。
    pub fn same_origin(&self, other: &Self) -> Result<bool, SupplyError> {
        let store = self
            .source
            .source
            .same_origin(&other.source.source)
            .map_err(super::problem)?;
        let audit = self
            .audit
            .same_origin(&other.audit)
            .map_err(|_| SupplyError::StoreUnavailable)?;
        Ok(store && audit)
    }
    /// 接入方先于人侧取得独占许可，遵循实际嵌套调用方向。
    pub fn quiesce(&self, deadline: Instant) -> Result<AuditQuiescence, SupplyError> {
        self.audit
            .quiesce(deadline)
            .map_err(|_| SupplyError::StoreUnavailable)
    }
    /// 七 owner 中 AgentStore 的固定位置；Audit 仍等最后冻结。
    pub fn freeze_store(&self, deadline: Instant) -> Result<FrozenAgentStore, SupplyError> {
        self.source.freeze(deadline)
    }
    /// Audit 阶段冻结这一个身份根绑定的真实附属账本。
    pub fn freeze_audit(&self, deadline: Instant) -> Result<FrozenAudit, SupplyError> {
        self.audit
            .freeze(deadline)
            .map_err(|_| SupplyError::StoreUnavailable)
    }
}
