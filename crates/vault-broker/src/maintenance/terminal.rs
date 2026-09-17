//! 只从原请求的精确持久终态补维护结果；当前文件 hash 永不构成成功证据。
use super::{
    record::{Slot, Subject},
    *,
};
use crate::{
    mutation::{self, record::Outcome, replay::Replay},
    recorded::store as requests,
};
pub(super) fn derive(broker: &FsVaultBroker, slot: &mut Slot) -> Result<bool, BrokerError> {
    let Some(r) = &mut slot.record else {
        return Ok(false);
    };
    if !r.executing {
        return Ok(false);
    }
    if let Subject::FileOperation { review } = &r.subject {
        let Some((resolution, evidence_id)) =
            crate::file_operation::maintenance::resolution(broker, review, r.intent.decision)?
        else {
            if r.outcome.is_some() {
                return Err(BrokerError::CorruptWriterState);
            }
            return Ok(false);
        };
        let mut result = super::outcome(&r.intent, MaintenanceOutcome::Applied);
        result.resolution = Some(resolution);
        result.evidence_id = evidence_id;
        if let Some(existing) = &r.outcome {
            return if existing == &result {
                Ok(false)
            } else {
                Err(BrokerError::CorruptWriterState)
            };
        }
        r.outcome = Some(result);
        return Ok(true);
    }
    let (key, fingerprint, target, reference, path, precondition) = match &r.subject {
        Subject::FileOperation { .. } => return Err(BrokerError::CorruptWriterState),
        Subject::Mutation { operation: op } => (
            op.key_hash.clone(),
            op.fingerprint.clone(),
            op.target_revision,
            op.recording.clone(),
            op.path.clone(),
            op.precondition,
        ),
        Subject::Restore { reference } => {
            let e = requests::load(broker, reference)?;
            (
                e.key_hash,
                e.fingerprint,
                e.target,
                Some(reference.clone()),
                e.path,
                e.precondition,
            )
        }
    };
    let Some(replay) = mutation::replay::load(&broker.data, &key, slot.pair_id)? else {
        if r.outcome.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        return Ok(false);
    };
    let Replay::Current(terminal) = replay else {
        return Err(BrokerError::CorruptWriterState);
    };
    if terminal.fingerprint != fingerprint
        || terminal.expected_revision != target
        || terminal.recording != reference
        || terminal.sequence.is_some()
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let mut result = super::outcome(&r.intent, MaintenanceOutcome::Applied);
    match (&terminal.outcome, r.intent.decision, &r.subject) {
        (
            Outcome::Cancelled {
                path: p,
                precondition: c,
            },
            MaintenanceDecision::CancelPrepared | MaintenanceDecision::CancelRestoreIntent,
            _,
        ) if p == &path && c == &precondition => {
            result.resolution = Some(crate::ReconciliationOutcome::Cancelled);
        }
        (
            Outcome::Indeterminate {
                path: p,
                precondition: c,
                evidence_id,
                current_revision,
            },
            MaintenanceDecision::PreserveInstalling,
            Subject::Mutation { operation },
        ) if p == &path && c == &precondition => {
            let observed = match r.intent.target.current {
                MaintenanceCurrent::Present { revision } => Some(revision),
                MaintenanceCurrent::Missing => None,
                MaintenanceCurrent::NotObserved | MaintenanceCurrent::Unobservable { .. } => {
                    return Err(BrokerError::CorruptWriterState);
                }
            };
            if observed != *current_revision {
                return Err(BrokerError::CorruptWriterState);
            }
            crate::recovery::maintenance::verify_evidence(
                broker,
                evidence_id,
                operation,
                observed,
            )?;
            result.resolution = Some(crate::ReconciliationOutcome::Indeterminate);
            result.evidence_id = Some(evidence_id.clone());
        }
        _ => return Err(BrokerError::CorruptWriterState),
    }
    broker.writer.check(&broker.root, &broker.data)?;
    if let Some(existing) = &r.outcome {
        if existing != &result {
            return Err(BrokerError::CorruptWriterState);
        }
        return Ok(false);
    }
    r.outcome = Some(result);
    Ok(true)
}
