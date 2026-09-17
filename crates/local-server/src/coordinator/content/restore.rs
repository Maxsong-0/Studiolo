//! 单页恢复分两阶段：持久固定意图、权威ACK、最终复验，然后才允许CAS安装。
use super::*;
use std::sync::atomic::Ordering;
use studiolo_vault_broker::RecordedRestore;

impl CanonicalWorkbench {
    pub(super) fn restore(
        &self,
        broker: &mut FsVaultBroker,
        principal: &ContentPrincipal,
        mutation: Mutation,
        metadata: MutationMetadata,
        input: &Replace,
        epoch: u64,
    ) -> Result<MutationReceipt, Problem> {
        let control = self.content_gate(principal, epoch)?;
        let ticket = broker
            .prepare_recorded_restore(mutation, metadata)
            .map_err(|error| mutation_error(error, input))?;
        self.core
            .content_restore_pending
            .store(true, Ordering::Release);
        drop(control);
        let acknowledged = self
            .core
            .content_audit
            .as_ref()
            .ok_or(())
            .and_then(|ledger| crate::editing::audit::append(ledger.as_ref(), ticket.intent()))
            .and_then(|(offset, hash)| {
                broker
                    .confirm_recorded_restore_intent(&ticket, offset, &hash)
                    .map_err(|_| ())
            });
        if acknowledged.is_err() {
            return Err(self.cancel_restore(broker, ticket, audit_unavailable()));
        }
        let control = match self.content_gate(principal, epoch) {
            Ok(control) => control,
            Err(error) => return Err(self.cancel_restore(broker, ticket, error)),
        };
        self.core.mark_stale(&control, "changed");
        let result = broker.commit_recorded_restore(ticket);
        drop(control);
        let _ = self.core.request("changed");
        let status = self.core.flush_content_audit(broker);
        if status.map_or(true, |status| status.restore_unconfirmed)
            || broker
                .write_status()
                .map_or(true, |status| status.unresolved)
        {
            return Err(Problem::unknown());
        }
        result.map_err(|error| mutation_error(error, input))
    }

    fn cancel_restore(
        &self,
        broker: &mut FsVaultBroker,
        ticket: RecordedRestore,
        known_failure: Problem,
    ) -> Problem {
        let cancelled = broker.cancel_recorded_restore(ticket).is_ok();
        let _ = self.core.flush_content_audit(broker);
        if cancelled {
            known_failure
        } else {
            Problem::unknown()
        }
    }
}
