//! 审阅固定后才占用唯一维护槽；重试不能改写原决定或操作者。
use super::{record::Active, *};
use crate::mutation;
impl FsVaultBroker {
    /// 只读取固定维护槽，校验真实 OS 身份、根、绑定与槽完整性。
    /// 不读取原 canonical 或 stage；返回的 target 是原意图证据，绝非新的目标观察。
    /// 用于目标不可读后的审计补记或取消未执行维护，不执行原决定。
    pub fn maintenance_pending(&self) -> Result<Option<MaintenancePending>, BrokerError> {
        MaintenanceActor::current()?;
        Ok(store::load(self)?.and_then(|slot| slot.pending()))
    }

    /// 返回有界逻辑审阅；维护槽自身使用独立 maintenance_id。
    pub fn maintenance_inspect(&self, limit: usize) -> Result<MaintenanceStatus, BrokerError> {
        super::limit(limit)?;
        let slot = store::load(self)?;
        let targets = target::reviews(self, limit)?
            .into_iter()
            .map(|r| r.target(self))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MaintenanceStatus {
            writer: self.write_status()?,
            targets,
            pending: slot.and_then(|s| s.pending()),
        })
    }
    /// 只固定维护意图；必须先经过外层 TTY 审阅和真实 OS 身份验证。
    pub fn prepare_maintenance(
        &mut self,
        review_id: &str,
        decision: MaintenanceDecision,
        actor: MaintenanceActor,
    ) -> Result<MaintenanceTicket, BrokerError> {
        valid_id(review_id)?;
        check_actor(&actor)?;
        let existing = store::load(self)?;
        if let Some(slot) = existing.as_ref().filter(|s| s.active()) {
            let r = slot
                .record
                .as_ref()
                .ok_or(BrokerError::CorruptWriterState)?;
            if r.intent.target.review_id != review_id
                || r.intent.decision != decision
                || r.intent.actor != actor
            {
                return Err(BrokerError::IdempotencyConflict);
            }
            return run::ticket(self, slot);
        }
        let (review, observed) =
            target::find(self, review_id)?.ok_or(BrokerError::CaptureChanged)?;
        if let target::Review::FileOperation(review) = &review {
            crate::file_operation::maintenance::ensure_preservable(review, decision)?;
        }
        if !target::allowed(observed.kind, decision) {
            return Err(BrokerError::InvalidArtifact(
                "维护动作不适用于审阅阶段".into(),
            ));
        }
        if matches!(
            decision,
            MaintenanceDecision::PreserveInstalling | MaintenanceDecision::CancelPrepared
        ) && observed.file_operation_id.is_none()
            && !observed.proposed_available
        {
            return Err(BrokerError::MutationUnresolved);
        }
        let mut slot = store::enable(self)?;
        let (review, target) = target::find(self, review_id)?.ok_or(BrokerError::CaptureChanged)?;
        slot.generation = slot
            .generation
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        let mut intent = MaintenanceAuditEvent {
            schema_version: if target.file_operation.is_some() {
                Some(3)
            } else {
                target.file_operation_id.as_ref().map(|_| 2)
            },
            event_id: String::new(),
            phase: ContentAuditPhase::Intent,
            vault_uuid: self.vault_uuid,
            actor,
            decision,
            requested_at_ms: crate::history::store::now(),
            target,
            outcome: None,
            resolution: None,
            evidence_id: None,
        };
        let subject = review.subject()?;
        slot.version = intent.schema_version.unwrap_or(1);
        intent.event_id =
            super::record::event_id(slot.pair_id, slot.generation, &intent, &subject)?;
        slot.record = Some(Active {
            intent,
            subject,
            intent_ack: None,
            executing: false,
            outcome: None,
            outcome_ack: None,
        });
        store::save(self, &slot)?;
        mutation::fault(self, 32)?;
        run::ticket(self, &slot)
    }
}
