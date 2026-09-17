//! 保全包记录观察到的证据和缺口；不把 current/隔离区 Missing 推成历史成功。
use super::{
    disk::{self, Folder},
    maintenance_review::{Review, Snapshot},
};
use crate::{
    BrokerError, FsVaultBroker,
    blob::{file, store as blobs},
    writer::directory,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    version: u32,
    review: Review,
}
pub(crate) fn preserve(broker: &FsVaultBroker, review: &Review) -> Result<String, BrokerError> {
    if review.active.rename.is_some() {
        return super::rename_evidence::preserve(broker, review);
    }
    super::maintenance::ensure_preservable(review, crate::MaintenanceDecision::PreserveInstalling)?;
    super::maintenance_review::check(broker, review)?;
    let evidence = Evidence {
        version: 1,
        review: review.clone(),
    };
    let id = super::digest(&evidence)?;
    let snapshots = directory(&broker.data, "snapshots")?;
    let history = directory(&broker.data, "history")?;
    let quarantine = super::store::required(broker)?.quarantine;
    let stages = directory(&broker.root, ".workbench/staging")?;
    let active = &review.active;
    let revision = active
        .history
        .as_ref()
        .and_then(|v| v.revision)
        .ok_or(BrokerError::CorruptWriterState)?;
    let mut total = 0u64;
    for observation in [
        &review.current,
        &review.original,
        &review.quarantine,
        &review.stage,
    ] {
        if let Snapshot::Present { proof } = observation {
            total = total
                .checked_add(proof.identity.bytes)
                .ok_or(BrokerError::WriterLimitExceeded)?;
        }
    }
    super::capacity::free(broker, total)?;
    for (role, root, name, observed) in [
        (
            "current",
            &broker.root,
            active.frozen.path.as_str().to_owned(),
            &review.current,
        ),
        ("history", &history, revision.to_string(), &review.original),
        (
            "quarantine",
            &quarantine,
            format!("{}.object", active.frozen.operation_id),
            &review.quarantine,
        ),
        (
            "stage",
            &stages,
            active
                .stage
                .as_ref()
                .map_or_else(String::new, |stage| stage.name.clone()),
            &review.stage,
        ),
    ] {
        if let Snapshot::Present { proof } = observed {
            let revision = proof.revision.ok_or(BrokerError::CorruptWriterState)?;
            let preserved = blobs::copy(
                broker,
                root,
                &blobs::logical(&name)?,
                &snapshots,
                &format!("file-operation-{id}-{role}.bin"),
                (&active.frozen.path, revision),
                &proof.identity,
            )?;
            if preserved.identity != proof.identity {
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
        &evidence,
        None,
    )?;
    verify(broker, &id, review)?;
    Ok(id)
}
pub(crate) fn verify(
    broker: &FsVaultBroker,
    id: &str,
    expected: &Review,
) -> Result<(), BrokerError> {
    if expected.active.rename.is_some() {
        return super::rename_evidence::verify(broker, id, expected);
    }
    let bytes = disk::read(broker, Folder::Evidence, &format!("{id}.json"))?
        .ok_or(BrokerError::CorruptWriterState)?;
    let evidence: Evidence = crate::writer::disk::decode(&bytes)?;
    if evidence.version != 1
        || super::digest(&evidence)? != id
        || super::digest(&evidence.review)? != super::digest(expected)?
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let snapshots = directory(&broker.data, "snapshots")?;
    for (role, observed) in [
        ("current", &expected.current),
        ("history", &expected.original),
        ("quarantine", &expected.quarantine),
        ("stage", &expected.stage),
    ] {
        if let Snapshot::Present { proof } = observed {
            let actual = blobs::verify(
                broker,
                &snapshots,
                &format!("file-operation-{id}-{role}.bin"),
                &expected.active.frozen.path,
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
