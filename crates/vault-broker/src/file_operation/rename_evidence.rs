//! 六角色证据各自绑定正确逻辑路径；完整保全/回读后才发布固定 manifest。
use super::{
    disk::{self, Folder},
    maintenance_review::{Review, Snapshot},
    *,
};
use crate::{
    BrokerError, FsVaultBroker, VaultPath,
    blob::{file, store as blobs},
    writer::directory,
};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    version: u32,
    review: Review,
}
const ROLES: [&str; 6] = [
    "source_current",
    "target_current",
    "source_original",
    "target_proposed",
    "source_quarantine",
    "target_stage",
];
fn selections(review: &Review) -> Result<[(&str, &Snapshot, &VaultPath); 6], BrokerError> {
    let s = review
        .rename
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let (to, _) = rename_model::get(&review.active)?;
    let from = &review.active.frozen.path;
    Ok([
        (ROLES[0], &review.current, from),
        (ROLES[1], &s.current, to),
        (ROLES[2], &review.original, from),
        (ROLES[3], &s.proposed, to),
        (ROLES[4], &review.quarantine, from),
        (ROLES[5], &review.stage, to),
    ])
}
pub(crate) fn preserve(broker: &FsVaultBroker, review: &Review) -> Result<String, BrokerError> {
    rename_review::preservable(review, crate::MaintenanceDecision::PreserveInstalling)?;
    super::maintenance_review::check(broker, review)?;
    let value = Evidence {
        version: 2,
        review: review.clone(),
    };
    let id = super::digest(&value)?;
    let snapshots = directory(&broker.data, "snapshots")?;
    let history = directory(&broker.data, "history")?;
    let quarantine = super::store::required(broker)?.quarantine;
    let stages = directory(&broker.root, ".workbench/staging")?;
    let a = &review.active;
    let (to, r) = rename_model::get(a)?;
    let source = a
        .history
        .as_ref()
        .and_then(|p| p.revision)
        .ok_or(BrokerError::CorruptWriterState)?;
    let target = r
        .history
        .as_ref()
        .and_then(|p| p.revision)
        .ok_or(BrokerError::CorruptWriterState)?;
    let stage = a.stage.as_ref().ok_or(BrokerError::CorruptWriterState)?;
    let selections = selections(review)?;
    let roots = [
        &broker.root,
        &broker.root,
        &history,
        &history,
        &quarantine,
        &stages,
    ];
    let names = [
        a.frozen.path.as_str().to_owned(),
        to.as_str().to_owned(),
        source.to_string(),
        target.to_string(),
        format!("{}.object", a.frozen.operation_id),
        stage.name.clone(),
    ];
    let total = selections.iter().try_fold(0u64, |n, (_, s, _)| match s {
        Snapshot::Present { proof } => n
            .checked_add(proof.identity.bytes)
            .ok_or(BrokerError::WriterLimitExceeded),
        _ => Ok(n),
    })?;
    super::capacity::free(broker, total)?;
    for (((role, s, path), root), name) in selections.into_iter().zip(roots).zip(names) {
        if let Snapshot::Present { proof } = s {
            let revision = proof.revision.ok_or(BrokerError::CorruptWriterState)?;
            let copied = blobs::copy(
                broker,
                root,
                &blobs::logical(&name)?,
                &snapshots,
                &format!("file-operation-{id}-{role}.bin"),
                (path, revision),
                &proof.identity,
            )?;
            if copied.identity != proof.identity {
                return Err(BrokerError::CaptureChanged);
            }
            file::check(root, &blobs::logical(&name)?, &proof.node)?;
        }
    }
    super::maintenance_review::check(broker, review)?;
    disk::save(
        broker,
        Folder::Evidence,
        &format!("{id}.json"),
        &value,
        None,
    )?;
    verify(broker, &id, review)?;
    Ok(id)
}
pub(crate) fn verify(broker: &FsVaultBroker, id: &str, review: &Review) -> Result<(), BrokerError> {
    let bytes = disk::read(broker, Folder::Evidence, &format!("{id}.json"))?
        .ok_or(BrokerError::CorruptWriterState)?;
    let value: Evidence = crate::writer::disk::decode(&bytes)?;
    if value.version != 2
        || super::digest(&value)? != id
        || super::digest(&value.review)? != super::digest(review)?
    {
        return Err(BrokerError::CorruptWriterState);
    }
    rename_review::preservable(review, crate::MaintenanceDecision::PreserveInstalling)?;
    let snapshots = directory(&broker.data, "snapshots")?;
    for (role, s, path) in selections(review)? {
        if let Snapshot::Present { proof } = s {
            let actual = blobs::verify(
                broker,
                &snapshots,
                &format!("file-operation-{id}-{role}.bin"),
                path,
                proof.revision.ok_or(BrokerError::CorruptWriterState)?,
            )?;
            if actual.identity != proof.identity {
                return Err(BrokerError::CorruptWriterState);
            }
        }
    }
    super::store::required(broker)?;
    Ok(())
}
