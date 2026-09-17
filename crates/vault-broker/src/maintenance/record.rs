//! 唯一持久槽；保留最后完成代次，旧票据不能消费下一次维护。
use super::*;
use crate::{mutation::record::Operation, recorded::store::RecordingRef, writer::disk};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Subject {
    Mutation {
        operation: Box<Operation>,
    },
    Restore {
        reference: RecordingRef,
    },
    FileOperation {
        review: Box<crate::file_operation::maintenance::Review>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ack {
    pub offset: u64,
    pub hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Active {
    pub intent: MaintenanceAuditEvent,
    pub subject: Subject,
    pub intent_ack: Option<Ack>,
    pub executing: bool,
    pub outcome: Option<MaintenanceAuditEvent>,
    pub outcome_ack: Option<Ack>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Slot {
    pub version: u32,
    pub pair_id: crate::VaultUuid,
    pub generation: u64,
    pub record: Option<Active>,
}
impl Slot {
    pub fn active(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|r| r.outcome_ack.is_none())
    }
    pub fn pending(&self) -> Option<MaintenancePending> {
        self.record
            .as_ref()
            .filter(|_| self.active())
            .map(|r| MaintenancePending {
                maintenance_id: r.intent.event_id.clone(),
                intent: r.intent.clone(),
                intent_confirmed: r.intent_ack.is_some(),
                execution_started: r.executing,
                outcome: r.outcome.clone(),
                outcome_confirmed: r.outcome_ack.is_some(),
            })
    }
    pub fn validate(&self, broker: &FsVaultBroker) -> Result<(), BrokerError> {
        if !matches!(self.version, 1..=3)
            || self.pair_id != broker.writer.binding.pair_id
            || (self.generation == 0) != self.record.is_none()
        {
            return Err(BrokerError::CorruptWriterState);
        }
        let Some(r) = &self.record else {
            if self.version != 1 {
                return Err(BrokerError::CorruptWriterState);
            }
            return Ok(());
        };
        let event = &r.intent;
        let typed = matches!(r.subject, Subject::FileOperation { .. });
        let rename = matches!(&r.subject, Subject::FileOperation { review } if review.active.rename.is_some());
        let expected_version = if rename {
            3
        } else if typed {
            2
        } else {
            1
        };
        if self.version != expected_version
            || event.schema_version != typed.then_some(expected_version)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if event.event_id != event_id(self.pair_id, self.generation, event, &r.subject)?
            || event.vault_uuid != broker.vault_uuid
            || event.phase != ContentAuditPhase::Intent
            || event.outcome.is_some()
            || event.resolution.is_some()
            || event.evidence_id.is_some()
            || event.requested_at_ms.is_some_and(|v| v < 0)
            || !disk::hex(&event.target.review_id)
            || !disk::hex(&event.target.request_fingerprint)
            || event.target.path.is_protected()
            || (r.executing && r.intent_ack.is_none())
            || (r.outcome.is_some() && r.intent_ack.is_none())
            || (r.outcome_ack.is_some() && r.outcome.is_none())
        {
            return Err(BrokerError::CorruptWriterState);
        }
        for ack in [&r.intent_ack, &r.outcome_ack].into_iter().flatten() {
            if ack.offset == 0 || !disk::hex(&ack.hash) {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        target::validate(broker, &r.subject, &event.target, event.decision)?;
        if let Some(result) = &r.outcome {
            let kind = result.outcome.ok_or(BrokerError::CorruptWriterState)?;
            let mut expected = super::outcome(event, kind);
            if kind == MaintenanceOutcome::Applied {
                if !r.executing {
                    return Err(BrokerError::CorruptWriterState);
                }
                expected.resolution = Some(match event.decision {
                    MaintenanceDecision::PreserveInstalling => {
                        crate::ReconciliationOutcome::Indeterminate
                    }
                    _ => crate::ReconciliationOutcome::Cancelled,
                });
                if event.decision == MaintenanceDecision::PreserveInstalling {
                    let id = result
                        .evidence_id
                        .as_deref()
                        .ok_or(BrokerError::CorruptWriterState)?;
                    if !disk::hex(id) {
                        return Err(BrokerError::CorruptWriterState);
                    }
                    expected.evidence_id = Some(id.to_owned());
                }
            } else if r.executing {
                return Err(BrokerError::CorruptWriterState);
            }
            if result != &expected {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Ok(())
    }
}
pub(super) fn event_id(
    pair: crate::VaultUuid,
    generation: u64,
    event: &MaintenanceAuditEvent,
    subject: &Subject,
) -> Result<String, BrokerError> {
    let mut value = event.clone();
    value.event_id.clear();
    Ok(crate::sha256(
        &serde_json::to_vec(&(
            if event.schema_version == Some(3) {
                "studiolo-maintenance-v3"
            } else if matches!(subject, Subject::FileOperation { .. }) {
                "studiolo-maintenance-v2"
            } else {
                "studiolo-maintenance-v1"
            },
            pair,
            generation,
            value,
            subject,
        ))
        .map_err(|_| BrokerError::CorruptWriterState)?,
    ))
}
