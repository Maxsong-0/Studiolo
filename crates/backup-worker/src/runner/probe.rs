//! 唯一作业快照、完整节点树和全部字节都匹配后才提供数据已验证证据。
use super::*;
use inspect::{listing, plan, snapshot};
use studiolo_backup::worker::{job::RemoteProbe, restic::SnapshotRef, state::RemoteVerification};
use studiolo_core::checkpoint::CheckpointId;

impl AppendRunner {
    fn inspect_output(&self, argv: &[String]) -> Result<process::Output> {
        let output = self
            .runtime
            .capture(argv, None, self.source.deadline(), &|| {
                self.source.check_time()
            })?;
        if !output.success {
            return Err(ProcessError::Protocol);
        }
        Ok(output)
    }
    fn locate_snapshot(&self, key: &JobKey) -> Result<Option<SnapshotRef>> {
        if key != self.key() {
            return Err(ProcessError::Source);
        }
        self.source.check()?;
        let argv = plan::snapshots(self.source.repository(), key)?;
        let output = self.inspect_output(&argv)?;
        let found = snapshot::locate(
            &output.stdout,
            key,
            &self.sealed().ticket().transfer_source.root,
        )?;
        self.source.check_time()?;
        Ok(found)
    }
    fn verify_snapshot(
        &self,
        snapshot: &SnapshotRef,
        checkpoint: &CheckpointId,
    ) -> Result<RemoteVerification> {
        self.verified.borrow_mut().take();
        if checkpoint != &self.key().checkpoint_id
            || self.locate_snapshot(self.key())?.as_ref() != Some(snapshot)
        {
            return Err(ProcessError::Source);
        }
        let output = self.inspect_output(&plan::listing(self.source.repository(), snapshot)?)?;
        listing::verify(
            &output.stdout,
            listing::Expected {
                key: self.key(),
                snapshot,
                root: &self.sealed().ticket().transfer_source.root,
                inventory: self.sealed().inventory(),
                producer: self.source.producer(),
                group: self.source.group(),
            },
        )?;
        self.verify_snapshot_files(snapshot)?;
        if self.locate_snapshot(self.key())?.as_ref() != Some(snapshot) {
            return Err(ProcessError::Source);
        }
        let verified_at = self.source.current_time()?;
        let verification = RemoteVerification {
            objects_complete: true,
            checksum_verified: true,
            paired_checkpoint_reference: true,
            verified_at,
        };
        let draft = FinalManifestDraft::new(
            self.key().clone(),
            self.sealed(),
            snapshot.clone(),
            verification,
        )
        .map_err(|_| ProcessError::Protocol)?;
        *self.verified.borrow_mut() = Some(draft);
        Ok(verification)
    }
}
impl RemoteProbe for AppendRunner {
    fn locate(&self, key: &JobKey) -> std::result::Result<Option<SnapshotRef>, ResticError> {
        self.locate_snapshot(key)
            .map_err(|_| ResticError::ExecutionFailed)
    }
    fn verify(
        &self,
        snapshot: &SnapshotRef,
        checkpoint_id: &CheckpointId,
    ) -> std::result::Result<RemoteVerification, ResticError> {
        self.verify_snapshot(snapshot, checkpoint_id)
            .map_err(|_| ResticError::ExecutionFailed)
    }
}
