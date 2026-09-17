//! 维护审阅显式区分缺失、可读原字节及不可观测；不可观测永远不是删除成功证据。
use super::{FileOperationKind, FileOperationPhase, FilePresence, record::Active};
use crate::{
    BrokerError, FsVaultBroker, MaintenanceCurrent, MaintenanceTarget, MaintenanceTargetKind,
    MaintenanceUnavailableReason,
    blob::{file, store as blobs},
    writer::directory,
};
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Snapshot {
    NotObserved,
    Missing,
    Present {
        proof: file::ContentProof,
    },
    Unobservable {
        reason: MaintenanceUnavailableReason,
    },
}
impl Snapshot {
    pub(crate) fn current(&self) -> Result<MaintenanceCurrent, BrokerError> {
        Ok(match self {
            Self::NotObserved => MaintenanceCurrent::NotObserved,
            Self::Missing => MaintenanceCurrent::Missing,
            Self::Present { proof } => MaintenanceCurrent::Present {
                revision: proof.revision.ok_or(BrokerError::CorruptWriterState)?,
            },
            Self::Unobservable { reason } => MaintenanceCurrent::Unobservable { reason: *reason },
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub active: Active,
    pub current: Snapshot,
    pub original: Snapshot,
    pub quarantine: Snapshot,
    pub stage: Snapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rename: Option<super::rename_review::Secondary>,
}
pub(crate) fn observe(
    broker: &FsVaultBroker,
    root: &Dir,
    name: &str,
    path: &crate::VaultPath,
) -> Result<Snapshot, BrokerError> {
    broker.writer.check(&broker.root, &broker.data)?;
    let source = blobs::logical(name)?;
    let result = (|| {
        if let Some(parents) = file::missing(root, &source)? {
            if file::missing(root, &source)?.as_ref() != Some(&parents) {
                return Err(BrokerError::CaptureChanged);
            }
            return Ok(Snapshot::Missing);
        }
        file::scan(
            root,
            &source,
            crate::blob::OBJECT_LIMIT,
            Some((broker.vault_uuid, path)),
        )
        .map(|proof| Snapshot::Present { proof })
    })();
    broker.writer.check(&broker.root, &broker.data)?;
    match result {
        Ok(value) => Ok(value),
        Err(
            BrokerError::NotFound(_)
            | BrokerError::UnsafeNode { .. }
            | BrokerError::PathCollision { .. }
            | BrokerError::UnstableRead(_)
            | BrokerError::CaptureChanged,
        ) => Ok(Snapshot::Unobservable {
            reason: MaintenanceUnavailableReason::UnsafeOrUnstable,
        }),
        Err(BrokerError::WriterLimitExceeded) => Ok(Snapshot::Unobservable {
            reason: MaintenanceUnavailableReason::LimitExceeded,
        }),
        Err(BrokerError::Io { .. }) => Ok(Snapshot::Unobservable {
            reason: MaintenanceUnavailableReason::IoUnavailable,
        }),
        Err(error) => Err(error),
    }
}
pub(crate) fn capture(broker: &FsVaultBroker) -> Result<Option<Review>, BrokerError> {
    let state = crate::mutation::load(broker)?;
    let Some(active) = state.file_operation else {
        return Ok(None);
    };
    if active.rename.is_some() {
        return super::rename_review::capture(broker, active);
    }
    if active.rejected
        || !matches!(
            active.phase,
            FileOperationPhase::Preparing
                | FileOperationPhase::Prepared
                | FileOperationPhase::DetachIssued
                | FileOperationPhase::InstallIssued
        )
    {
        return Ok(None);
    }
    if matches!(
        active.phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    ) {
        return Ok(Some(Review {
            active,
            current: Snapshot::NotObserved,
            original: Snapshot::NotObserved,
            quarantine: Snapshot::NotObserved,
            stage: Snapshot::NotObserved,
            rename: None,
        }));
    }
    let path = &active.frozen.path;
    let revision = active
        .history
        .as_ref()
        .and_then(|p| p.revision)
        .ok_or(BrokerError::CorruptWriterState)?;
    let history = directory(&broker.data, "history")?;
    let (quarantine, stage) = match active.frozen.kind {
        FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
        FileOperationKind::Delete { .. } => {
            let root = super::store::required(broker)?.quarantine;
            (
                observe(
                    broker,
                    &root,
                    &format!("{}.object", active.frozen.operation_id),
                    path,
                )?,
                Snapshot::NotObserved,
            )
        }
        FileOperationKind::RestoreTombstone { .. } => {
            let root = directory(&broker.root, ".workbench/staging")?;
            let stage = active
                .stage
                .as_ref()
                .ok_or(BrokerError::CorruptWriterState)?;
            (
                Snapshot::NotObserved,
                observe(broker, &root, &stage.name, path)?,
            )
        }
    };
    let result = Review {
        current: observe(broker, &broker.root, path.as_str(), path)?,
        original: observe(broker, &history, &revision.to_string(), path)?,
        quarantine,
        stage,
        rename: None,
        active,
    };
    let after = crate::mutation::load(broker)?;
    if super::digest(&after.file_operation)? != super::digest(&Some(&result.active))? {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(Some(result))
}
pub(crate) fn target(review: &Review) -> Result<MaintenanceTarget, BrokerError> {
    if review.active.rename.is_some() {
        return super::rename_review::target(review);
    }
    if review.rename.is_some() {
        return Err(BrokerError::CorruptWriterState);
    }
    validate(review)?;
    let active = &review.active;
    let prepared = matches!(
        active.phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    );
    let previous = match active.before.presence()? {
        FilePresence::Present { revision, .. } => Some(revision),
        _ => None,
    };
    let revision = match active.frozen.kind {
        FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
        FileOperationKind::Delete { .. } => None,
        FileOperationKind::RestoreTombstone { .. } => {
            active.history.as_ref().and_then(|v| v.revision)
        }
    };
    Ok(MaintenanceTarget {
        review_id: super::digest(&("file-operation-maintenance-review-v1", review))?,
        request_fingerprint: active.frozen.fingerprint.clone(),
        path: active.frozen.path.clone(),
        kind: if prepared {
            MaintenanceTargetKind::Prepared
        } else {
            MaintenanceTargetKind::Installing
        },
        sequence: Some(active.sequence),
        previous_revision: previous,
        target_revision: revision,
        file_operation_id: Some(active.frozen.operation_id.clone()),
        secondary: None,
        file_operation: None,
        current: review.current.current()?,
        proposed_available: prepared
            || matches!(active.frozen.kind, FileOperationKind::Delete { .. })
            || matches!(review.stage, Snapshot::Present { .. }),
    })
}
pub(crate) fn check(broker: &FsVaultBroker, review: &Review) -> Result<(), BrokerError> {
    let observed = capture(broker)?.ok_or(BrokerError::CaptureChanged)?;
    if super::digest(&observed)? != super::digest(review)? {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}

// 各动作只准三个实际对象；不能以同时声明 quarantine/stage 扩大保全预算。
fn validate(review: &Review) -> Result<(), BrokerError> {
    let inactive = |value: &Snapshot| matches!(value, Snapshot::NotObserved);
    if matches!(
        review.active.phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    ) {
        if [
            &review.current,
            &review.original,
            &review.quarantine,
            &review.stage,
        ]
        .into_iter()
        .all(inactive)
        {
            return Ok(());
        }
        return Err(BrokerError::CorruptWriterState);
    }
    let selected = match review.active.frozen.kind {
        FileOperationKind::Delete { .. } if inactive(&review.stage) => &review.quarantine,
        FileOperationKind::RestoreTombstone { .. } if inactive(&review.quarantine) => &review.stage,
        _ => return Err(BrokerError::CorruptWriterState),
    };
    for value in [&review.current, &review.original, selected] {
        match value {
            Snapshot::NotObserved => return Err(BrokerError::CorruptWriterState),
            Snapshot::Present { proof } => super::observation::Observation::Present {
                proof: proof.clone(),
            }
            .validate()?,
            _ => {}
        }
    }
    Ok(())
}
