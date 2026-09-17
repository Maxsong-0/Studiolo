//! Intent ACK 之前不协调；开始执行之后，异常保持未知而非猜一个无副作用结果。
use super::{record::Slot, target::Review, *};
use crate::mutation;
impl FsVaultBroker {
    /// 消费固定票据；未确认意图不会开始，已执行异常也不会被降格为未执行。
    pub fn commit_maintenance(
        &mut self,
        ticket: MaintenanceTicket,
    ) -> Result<MaintenanceReceipt, BrokerError> {
        check_actor(&ticket.intent.actor)?;
        let mut slot = check_ticket(self, &ticket)?;
        if terminal::derive(self, &mut slot)? {
            store::save(self, &slot)?;
        }
        let r = slot
            .record
            .as_ref()
            .ok_or(BrokerError::CorruptWriterState)?;
        if r.outcome.is_some() {
            return receipt(self, &slot);
        }
        if r.intent_ack.is_none() {
            return Err(BrokerError::MutationUnresolved);
        }
        let found = target::find(self, &r.intent.target.review_id)?;
        let Some((review, observed)) = found else {
            if r.executing {
                return Err(BrokerError::MutationUnresolved);
            }
            return no_action(self, slot, MaintenanceOutcome::NotApplied);
        };
        if observed != r.intent.target {
            return Err(BrokerError::CaptureChanged);
        }
        let decision = r.intent.decision;
        if let Review::FileOperation(review) = &review
            && !crate::file_operation::maintenance::intent_confirmed(self, review)?
        {
            return Err(BrokerError::MutationUnresolved);
        }
        if let Review::FileOperation(review) = &review {
            crate::file_operation::maintenance::ensure_preservable(review, decision)?;
        }
        // proposed 在 Executing 之前取得，但原协调在消费审阅时仍重验 stage/current/journal。
        let proposed = match &review {
            Review::Mutation(r) if decision == MaintenanceDecision::PreserveInstalling => Some(
                r.maintenance_proposed(self)?
                    .ok_or(BrokerError::MutationUnresolved)?,
            ),
            _ => None,
        };
        slot.record
            .as_mut()
            .ok_or(BrokerError::CorruptWriterState)?
            .executing = true;
        store::save(self, &slot)?;
        mutation::fault(self, 33)?;
        let result = match review {
            Review::Mutation(r) => self
                .reconcile_maintenance_proposed(
                    &r,
                    match decision {
                        MaintenanceDecision::CancelPrepared => None,
                        MaintenanceDecision::PreserveInstalling => {
                            Some(proposed.ok_or(BrokerError::CorruptWriterState)?)
                        }
                        _ => return Err(BrokerError::CorruptWriterState),
                    },
                )
                .map(|_| ()),
            Review::Restore(e) => crate::recorded::cancel_for_maintenance(self, &e),
            Review::FileOperation(review) => {
                crate::file_operation::maintenance::apply(self, &review, decision)
            }
        };
        if result.is_err() {
            return Err(BrokerError::MutationUnresolved);
        }
        mutation::fault(self, 34)?;
        if !terminal::derive(self, &mut slot)? {
            return Err(BrokerError::MutationUnresolved);
        }
        store::save(self, &slot)?;
        mutation::fault(self, 35)?;
        receipt(self, &slot)
    }
    /// 只取消尚未执行的维护自身；原请求即使漂移也不被改写。
    pub fn cancel_maintenance(
        &mut self,
        maintenance_id: &str,
        actor: MaintenanceActor,
    ) -> Result<MaintenanceReceipt, BrokerError> {
        valid_id(maintenance_id)?;
        check_actor(&actor)?;
        let slot = store::load(self)?.ok_or(BrokerError::CaptureChanged)?;
        let r = slot.record.as_ref().ok_or(BrokerError::CaptureChanged)?;
        if r.intent.event_id != maintenance_id || r.intent.actor != actor {
            return Err(BrokerError::IdempotencyConflict);
        }
        if r.executing || r.intent_ack.is_none() {
            return Err(BrokerError::MutationUnresolved);
        }
        if let Some(event) = &r.outcome {
            return if event.outcome == Some(MaintenanceOutcome::Cancelled) {
                receipt(self, &slot)
            } else {
                Err(BrokerError::IdempotencyConflict)
            };
        }
        no_action(self, slot, MaintenanceOutcome::Cancelled)
    }
}
fn no_action(
    broker: &FsVaultBroker,
    mut slot: Slot,
    result: MaintenanceOutcome,
) -> Result<MaintenanceReceipt, BrokerError> {
    let r = slot
        .record
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    r.outcome = Some(super::outcome(&r.intent, result));
    store::save(broker, &slot)?;
    receipt(broker, &slot)
}
pub(super) fn ticket(
    broker: &FsVaultBroker,
    slot: &Slot,
) -> Result<MaintenanceTicket, BrokerError> {
    Ok(MaintenanceTicket {
        instance: Arc::clone(&broker.capture_instance),
        generation: slot.generation,
        intent: slot
            .record
            .as_ref()
            .ok_or(BrokerError::CorruptWriterState)?
            .intent
            .clone(),
    })
}
pub(super) fn check_ticket(
    broker: &FsVaultBroker,
    ticket: &MaintenanceTicket,
) -> Result<Slot, BrokerError> {
    if !Arc::ptr_eq(&broker.capture_instance, &ticket.instance) {
        return Err(BrokerError::CaptureChanged);
    }
    let slot = store::load(broker)?.ok_or(BrokerError::CaptureChanged)?;
    if slot.generation != ticket.generation
        || slot.record.as_ref().map(|r| &r.intent) != Some(&ticket.intent)
    {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(slot)
}
fn receipt(broker: &FsVaultBroker, slot: &Slot) -> Result<MaintenanceReceipt, BrokerError> {
    let event = slot
        .record
        .as_ref()
        .and_then(|r| r.outcome.clone())
        .ok_or(BrokerError::MutationUnresolved)?;
    Ok(MaintenanceReceipt {
        event,
        committed_through: mutation::load(broker)?.committed_through,
    })
}
