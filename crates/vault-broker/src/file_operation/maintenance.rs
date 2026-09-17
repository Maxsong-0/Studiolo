//! typed 原终态与维护自身终态分别确认；只执行维护槽已固定的取消/保全决定。
pub(crate) use super::maintenance_review::{Review, capture, target};
use super::*;
use crate::{
    BrokerError, FsVaultBroker, MaintenanceDecision, MaintenanceTarget, ReconciliationOutcome,
    mutation,
};

pub(crate) fn validate(
    broker: &FsVaultBroker,
    review: &Review,
    observed: &MaintenanceTarget,
    decision: MaintenanceDecision,
) -> Result<(), BrokerError> {
    review.active.validate(broker.writer.binding.pair_id)?;
    if target(review)? != *observed
        || !matches!(
            (observed.kind, decision),
            (
                crate::MaintenanceTargetKind::Prepared,
                MaintenanceDecision::CancelPrepared
            ) | (
                crate::MaintenanceTargetKind::Installing,
                MaintenanceDecision::PreserveInstalling
            )
        )
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
pub(crate) fn apply(
    broker: &FsVaultBroker,
    review: &Review,
    decision: MaintenanceDecision,
) -> Result<(), BrokerError> {
    ensure_preservable(review, decision)?;
    super::maintenance_review::check(broker, review)?;
    let request = request::ensure(broker, &review.active)?;
    if !audit::confirmed(broker, &audit::intent(&request))? {
        return Err(BrokerError::MutationUnresolved);
    }
    let (outcome, evidence) = match decision {
        MaintenanceDecision::CancelPrepared
            if matches!(
                review.active.phase,
                FileOperationPhase::Preparing | FileOperationPhase::Prepared
            ) =>
        {
            (FileOperationOutcome::Cancelled, None)
        }
        MaintenanceDecision::PreserveInstalling
            if matches!(
                review.active.phase,
                FileOperationPhase::DetachIssued
                    | FileOperationPhase::InstallIssued
                    | FileOperationPhase::TargetInstallIssued
                    | FileOperationPhase::TargetInstalled
                    | FileOperationPhase::SourceDetachIssued
            ) =>
        {
            (
                FileOperationOutcome::Indeterminate,
                Some(super::evidence::preserve(broker, review)?),
            )
        }
        _ => return Err(BrokerError::CorruptWriterState),
    };
    super::maintenance_review::check(broker, review)?;
    let mut state = mutation::load(broker)?;
    finish::set_terminal(&mut state, outcome, evidence)?;
    mutation::save(broker, &state)?;
    finish::resume(broker, &mut state)
}
pub(crate) fn resolution(
    broker: &FsVaultBroker,
    review: &Review,
    decision: MaintenanceDecision,
) -> Result<Option<(ReconciliationOutcome, Option<String>)>, BrokerError> {
    let Some(terminal) = terminal::load(broker, &review.active.frozen)? else {
        return Ok(None);
    };
    if terminal.receipt.sequence != review.active.sequence {
        return Err(BrokerError::CorruptWriterState);
    }
    match (decision, terminal.receipt.outcome) {
        (MaintenanceDecision::CancelPrepared, FileOperationOutcome::Cancelled) => {
            Ok(Some((ReconciliationOutcome::Cancelled, None)))
        }
        (MaintenanceDecision::PreserveInstalling, FileOperationOutcome::Indeterminate) => {
            let id = terminal
                .receipt
                .evidence_id
                .ok_or(BrokerError::CorruptWriterState)?;
            super::evidence::verify(broker, &id, review)?;
            Ok(Some((ReconciliationOutcome::Indeterminate, Some(id))))
        }
        _ => Err(BrokerError::CorruptWriterState),
    }
}
pub(crate) fn outcome_confirmed(
    broker: &FsVaultBroker,
    review: &Review,
) -> Result<bool, BrokerError> {
    let Some(terminal) = terminal::load(broker, &review.active.frozen)? else {
        return Ok(false);
    };
    let request = request::load(broker, &review.active.frozen.operation_id)?
        .ok_or(BrokerError::CorruptWriterState)?;
    audit::confirmed(broker, &audit::outcome(&request, &terminal)?)
}

pub(crate) fn intent_confirmed(
    broker: &FsVaultBroker,
    review: &Review,
) -> Result<bool, BrokerError> {
    let request = request::ensure(broker, &review.active)?;
    audit::confirmed(broker, &audit::intent(&request))
}

// stage 不可观测时不能解除其唯一活动引用；先恢复可观测性，再重新审阅。
pub(crate) fn ensure_preservable(
    review: &Review,
    decision: MaintenanceDecision,
) -> Result<(), BrokerError> {
    if review.active.rename.is_some() {
        return super::rename_review::preservable(review, decision);
    }
    if decision == MaintenanceDecision::PreserveInstalling
        && matches!(
            review.active.frozen.kind,
            FileOperationKind::RestoreTombstone { .. }
        )
        && matches!(
            review.stage,
            super::maintenance_review::Snapshot::Unobservable { .. }
        )
    {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
