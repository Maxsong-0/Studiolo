//! 不透明候选与固定槽代次绑定；ACK 只接受既定事件，不能重新选择动作。
use super::{record::Ack, *};
use crate::{mutation, writer::disk};
impl FsVaultBroker {
    /// 核对外层实际追加的完整维护 Intent 回执后持久确认；此方法不执行协调。
    pub fn confirm_maintenance_intent(
        &mut self,
        ticket: &MaintenanceTicket,
        offset: u64,
        hash: &str,
    ) -> Result<(), BrokerError> {
        let slot = run::check_ticket(self, ticket)?;
        self.confirm_maintenance_audit(
            MaintenanceAuditCandidate {
                instance: Arc::clone(&self.capture_instance),
                generation: slot.generation,
                event: ticket.intent.clone(),
            },
            offset,
            hash,
        )
    }
    /// 至多返回单槽的两个固定事件，Intent 在前；不开始未执行的决定。
    pub fn pending_maintenance_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<MaintenanceAuditCandidate>, BrokerError> {
        super::limit(limit)?;
        super::reopen(self)?;
        let Some(slot) = store::load(self)? else {
            return Ok(Vec::new());
        };
        let Some(r) = slot.record else {
            return Ok(Vec::new());
        };
        let mut events = Vec::new();
        if r.intent_ack.is_none() {
            events.push(r.intent);
        }
        if r.outcome_ack.is_none()
            && let Some(outcome) = r.outcome
        {
            events.push(outcome);
        }
        Ok(events
            .into_iter()
            .take(limit)
            .map(|event| MaintenanceAuditCandidate {
                instance: Arc::clone(&self.capture_instance),
                generation: slot.generation,
                event,
            })
            .collect())
    }
    /// 调用方已验证真实完整审计记录后消费；Outcome ACK 之前所有新写仍关闭。
    pub fn confirm_maintenance_audit(
        &mut self,
        candidate: MaintenanceAuditCandidate,
        offset: u64,
        hash: &str,
    ) -> Result<(), BrokerError> {
        if offset == 0 || !disk::hex(hash) {
            return Err(BrokerError::InvalidArtifact(
                "维护审计回执摘要不合法".into(),
            ));
        }
        if !Arc::ptr_eq(&self.capture_instance, &candidate.instance) {
            return Err(BrokerError::CaptureChanged);
        }
        let mut slot = store::load(self)?.ok_or(BrokerError::CaptureChanged)?;
        if slot.generation != candidate.generation {
            return Err(BrokerError::CaptureChanged);
        }
        let r = slot.record.as_mut().ok_or(BrokerError::CaptureChanged)?;
        let ack = Ack {
            offset,
            hash: hash.to_owned(),
        };
        let (event, previous) = match candidate.event.phase {
            ContentAuditPhase::Intent => (&r.intent, &mut r.intent_ack),
            ContentAuditPhase::Outcome => {
                if r.outcome
                    .as_ref()
                    .is_some_and(|event| event.outcome == Some(MaintenanceOutcome::Applied))
                    && let record::Subject::FileOperation { review } = &r.subject
                    && !crate::file_operation::maintenance::outcome_confirmed(self, review)?
                {
                    return Err(BrokerError::MutationUnresolved);
                }
                if r.intent_ack.is_none() {
                    return Err(BrokerError::MutationUnresolved);
                }
                (
                    r.outcome.as_ref().ok_or(BrokerError::MutationUnresolved)?,
                    &mut r.outcome_ack,
                )
            }
        };
        if event != &candidate.event {
            return Err(BrokerError::CaptureChanged);
        }
        if let Some(existing) = previous {
            return if *existing == ack {
                Ok(())
            } else {
                Err(BrokerError::IdempotencyConflict)
            };
        }
        *previous = Some(ack);
        store::save(self, &slot)?;
        mutation::fault(self, 36)?;
        Ok(())
    }
}
