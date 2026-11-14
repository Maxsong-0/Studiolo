use super::{listing, plan::Marker};
use crate::{
    ProcessError, Result,
    process::{DigestExpectation, Output},
    runner::inspect::{plan, snapshot},
};
use studiolo_backup::worker::{
    final_manifest::FinalManifestDraft,
    restic::{RepositoryRef, SnapshotRef},
};

pub(crate) fn read(
    draft: &FinalManifestDraft,
    repository: &RepositoryRef,
    metadata: &impl Fn(&[String]) -> Result<Output>,
    bytes: &impl Fn(&[String], DigestExpectation<'_>) -> Result<()>,
) -> Result<Option<SnapshotRef>> {
    let marker = Marker::for_job(draft.key())?;
    let locate = || {
        let output = metadata(&marker.snapshots(repository))?;
        if !output.success {
            return Err(ProcessError::Protocol);
        }
        snapshot::locate_named(&output.stdout, &marker.tag, &marker.path)
    };
    let Some(found) = locate()? else {
        return Ok(None);
    };
    if &found == draft.data_snapshot() {
        return Err(ProcessError::Protocol);
    }
    let output = metadata(&plan::listing(repository, &found)?)?;
    if !output.success {
        return Err(ProcessError::Protocol);
    }
    let encoded = draft.encode().map_err(|_| ProcessError::Protocol)?;
    listing::verify(&output.stdout, &marker, &found, encoded.len() as u64)?;
    let hash = draft.sha256().map_err(|_| ProcessError::Protocol)?;
    bytes(
        &marker.dump(repository, &found)?,
        DigestExpectation {
            bytes: encoded.len() as u64,
            sha256: &hash,
        },
    )?;
    if locate()?.as_ref() != Some(&found) {
        return Err(ProcessError::Protocol);
    }
    Ok(Some(found))
}
