//! 维护仅保留最后一代槽；核对该槽仍引用的全部原记录，不虚构已被替换的历史。
use super::store;
use crate::{
    BrokerError, FsVaultBroker,
    checkpoint::audit::{BrokerCheckpointAudit, Verifier},
};
pub(crate) fn verify(broker: &FsVaultBroker, verify: &mut Verifier<'_>) -> Result<(), BrokerError> {
    let Some(slot) = store::load(broker)? else {
        return Ok(());
    };
    if slot.active() {
        return Err(BrokerError::MutationUnresolved);
    }
    let Some(record) = slot.record else {
        return Ok(());
    };
    let intent = record.intent_ack.ok_or(BrokerError::MutationUnresolved)?;
    let outcome = record.outcome_ack.ok_or(BrokerError::MutationUnresolved)?;
    let result = record
        .outcome
        .as_ref()
        .ok_or(BrokerError::MutationUnresolved)?;
    verify(
        BrokerCheckpointAudit::Maintenance(&record.intent),
        intent.offset,
        &intent.hash,
    )?;
    verify(
        BrokerCheckpointAudit::Maintenance(result),
        outcome.offset,
        &outcome.hash,
    )
}
