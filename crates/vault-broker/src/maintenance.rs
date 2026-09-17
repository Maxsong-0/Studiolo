//! 离线 OS 维护的封闭决策：双审计确认、单持久槽，不授予应用身份。
mod audit;
mod model;
mod prepare;
mod record;
mod rename_model;
mod run;
pub(crate) mod store;
mod target;
mod terminal;
#[cfg(test)]
mod tests;

use crate::{BrokerError, ContentAuditPhase, FsVaultBroker};
pub use model::*;
pub use rename_model::*;
use std::sync::Arc;

/// 不透明同实例维护票据；不包含正文，也不能绕过审计准入。
pub struct MaintenanceTicket {
    instance: Arc<()>,
    generation: u64,
    intent: MaintenanceAuditEvent,
}
impl MaintenanceTicket {
    /// 固定维护意图，调用方必须实际提交并核对完整审计回执。
    pub fn intent(&self) -> &MaintenanceAuditEvent {
        &self.intent
    }
}

/// 固定事件候选；不能构造、反序列化或跨实例确认。
pub struct MaintenanceAuditCandidate {
    instance: Arc<()>,
    generation: u64,
    event: MaintenanceAuditEvent,
}
impl MaintenanceAuditCandidate {
    /// 待提交到 AuditLedger 的固定事件，不含正文和原请求键。
    pub fn event(&self) -> &MaintenanceAuditEvent {
        &self.event
    }
}

pub(crate) fn blocked(broker: &FsVaultBroker) -> Result<bool, BrokerError> {
    Ok(store::load(broker)?.is_some_and(|slot| slot.active()))
}
pub(crate) fn ensure_no_maintenance(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    if blocked(broker)? {
        Err(BrokerError::MutationUnresolved)
    } else {
        Ok(())
    }
}
pub(crate) fn ensure_idle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    ensure_no_maintenance(broker)?;
    if crate::mutation::load(broker)?.file_operation.is_some() {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
pub(crate) fn reopen(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    let Some(mut slot) = store::load(broker)? else {
        return Ok(());
    };
    if terminal::derive(broker, &mut slot)? {
        store::save(broker, &slot)?;
    }
    Ok(())
}
fn valid_id(value: &str) -> Result<(), BrokerError> {
    if crate::writer::disk::hex(value) {
        Ok(())
    } else {
        Err(BrokerError::InvalidArtifact("维护选择器不合法".into()))
    }
}
fn limit(value: usize) -> Result<(), BrokerError> {
    if (1..=64).contains(&value) {
        Ok(())
    } else {
        Err(BrokerError::WriterLimitExceeded)
    }
}
fn check_actor(actor: &MaintenanceActor) -> Result<(), BrokerError> {
    if actor == &MaintenanceActor::current()? {
        Ok(())
    } else {
        Err(BrokerError::InvalidArtifact(
            "维护主体不匹配实际 OS 身份".into(),
        ))
    }
}
fn outcome(intent: &MaintenanceAuditEvent, result: MaintenanceOutcome) -> MaintenanceAuditEvent {
    let mut event = intent.clone();
    event.phase = ContentAuditPhase::Outcome;
    event.outcome = Some(result);
    event
}
