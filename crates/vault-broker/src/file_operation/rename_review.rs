//! 双路径完整审阅；历史子效果来自固定 Active，绝不从当前两个名字推断。
use super::{
    maintenance_review::{Review, Snapshot, observe},
    observation::Observation,
    record::Active,
    *,
};
use crate::{
    BrokerError, FsVaultBroker, MaintenanceDecision, MaintenanceFileOperation,
    MaintenanceSecondaryTarget, MaintenanceTarget, MaintenanceTargetKind, writer::directory,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Secondary {
    pub current: Snapshot,
    pub proposed: Snapshot,
}
fn bad() -> BrokerError {
    BrokerError::CorruptWriterState
}
pub(crate) fn capture(
    broker: &FsVaultBroker,
    active: Active,
) -> Result<Option<Review>, BrokerError> {
    use FileOperationPhase::*;
    if active.rejected
        || !matches!(
            active.phase,
            Preparing | Prepared | TargetInstallIssued | TargetInstalled | SourceDetachIssued
        )
    {
        return Ok(None);
    }
    let review = if matches!(active.phase, Preparing | Prepared) {
        Review {
            active,
            current: Snapshot::NotObserved,
            original: Snapshot::NotObserved,
            quarantine: Snapshot::NotObserved,
            stage: Snapshot::NotObserved,
            rename: Some(Secondary {
                current: Snapshot::NotObserved,
                proposed: Snapshot::NotObserved,
            }),
        }
    } else {
        let (to, r) = rename_model::get(&active)?;
        let source = active
            .history
            .as_ref()
            .and_then(|p| p.revision)
            .ok_or_else(bad)?;
        let target = r
            .history
            .as_ref()
            .and_then(|p| p.revision)
            .ok_or_else(bad)?;
        let stage = active.stage.as_ref().ok_or_else(bad)?;
        let history = directory(&broker.data, "history")?;
        let stages = directory(&broker.root, ".workbench/staging")?;
        let quarantine = super::store::required(broker)?.quarantine;
        Review {
            current: observe(
                broker,
                &broker.root,
                active.frozen.path.as_str(),
                &active.frozen.path,
            )?,
            original: observe(broker, &history, &source.to_string(), &active.frozen.path)?,
            quarantine: observe(
                broker,
                &quarantine,
                &format!("{}.object", active.frozen.operation_id),
                &active.frozen.path,
            )?,
            stage: observe(broker, &stages, &stage.name, to)?,
            rename: Some(Secondary {
                current: observe(broker, &broker.root, to.as_str(), to)?,
                proposed: observe(broker, &history, &target.to_string(), to)?,
            }),
            active,
        }
    };
    let after = crate::mutation::load(broker)?;
    if super::digest(&after.file_operation)? != super::digest(&Some(&review.active))? {
        return Err(BrokerError::CaptureChanged);
    }
    validate(&review)?;
    Ok(Some(review))
}
pub(crate) fn validate(review: &Review) -> Result<(), BrokerError> {
    review.active.validate(review.active.frozen.pair_id)?;
    let r = review.rename.as_ref().ok_or_else(bad)?;
    let prepared = matches!(
        review.active.phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    );
    for s in [
        &review.current,
        &review.original,
        &review.quarantine,
        &review.stage,
        &r.current,
        &r.proposed,
    ] {
        if prepared != matches!(s, Snapshot::NotObserved) {
            return Err(bad());
        }
        if let Snapshot::Present { proof } = s {
            Observation::Present {
                proof: proof.clone(),
            }
            .validate()?;
        }
    }
    Ok(())
}
fn revision(value: &Observation) -> Result<Option<crate::Revision>, BrokerError> {
    Ok(match value.presence()? {
        FilePresence::Missing => None,
        FilePresence::Present { revision, .. } => Some(revision),
    })
}
fn complete(value: &Snapshot, expected: Option<&crate::blob::file::ContentProof>) -> bool {
    matches!(value,Snapshot::Present{proof} if expected.is_some_and(|p|p.identity==proof.identity&&p.revision==proof.revision))
}
pub(crate) fn target(review: &Review) -> Result<MaintenanceTarget, BrokerError> {
    validate(review)?;
    let a = &review.active;
    let (to, r) = rename_model::get(a)?;
    let s = review.rename.as_ref().ok_or_else(bad)?;
    let prepared = matches!(
        a.phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    );
    let ready = a.phase == FileOperationPhase::Prepared;
    let effects = if prepared {
        vec![
            FileEffect::NotApplied { path: to.clone() },
            FileEffect::NotApplied {
                path: a.frozen.path.clone(),
            },
        ]
    } else {
        rename_terminal::effects(a, FileOperationOutcome::Indeterminate)?
    };
    Ok(MaintenanceTarget {
        review_id: super::digest(&("file-operation-maintenance-review-v2", review))?,
        request_fingerprint: a.frozen.fingerprint.clone(),
        path: a.frozen.path.clone(),
        kind: if prepared {
            MaintenanceTargetKind::Prepared
        } else {
            MaintenanceTargetKind::Installing
        },
        sequence: Some(a.sequence),
        previous_revision: revision(&a.before)?,
        target_revision: None,
        file_operation_id: Some(a.frozen.operation_id.clone()),
        current: review.current.current()?,
        proposed_available: if prepared {
            ready
        } else {
            complete(&review.original, a.history.as_ref())
        },
        secondary: Some(MaintenanceSecondaryTarget {
            path: to.clone(),
            previous_revision: revision(&r.before)?,
            target_revision: if a.phase == FileOperationPhase::Preparing {
                None
            } else {
                r.target_revision
            },
            current: s.current.current()?,
            proposed_available: if prepared {
                ready
            } else {
                complete(&s.proposed, r.history.as_ref())
                    && !matches!(review.stage, Snapshot::Unobservable { .. })
            },
        }),
        file_operation: Some(MaintenanceFileOperation {
            kind: a.frozen.kind.clone(),
            phase: a.phase,
            effects,
        }),
    })
}
pub(crate) fn preservable(
    review: &Review,
    decision: MaintenanceDecision,
) -> Result<(), BrokerError> {
    validate(review)?;
    if decision != MaintenanceDecision::PreserveInstalling {
        return Ok(());
    }
    let r = review.rename.as_ref().ok_or_else(bad)?;
    for s in [
        &review.current,
        &review.original,
        &review.quarantine,
        &review.stage,
        &r.current,
        &r.proposed,
    ] {
        if matches!(s, Snapshot::NotObserved | Snapshot::Unobservable { .. }) {
            return Err(BrokerError::MutationUnresolved);
        }
    }
    let a = &review.active;
    let (_, p) = rename_model::get(a)?;
    if !complete(&review.original, a.history.as_ref()) || !complete(&r.proposed, p.history.as_ref())
    {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
