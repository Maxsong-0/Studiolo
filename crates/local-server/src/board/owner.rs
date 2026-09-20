//! 只借用现有 canonical owner；TaskEngine 不打开第二个 FsVaultBroker。
use crate::CanonicalWorkbench;
use std::sync::Arc;
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::*;

pub(super) struct SharedBroker {
    pub workbench: Arc<CanonicalWorkbench>,
    pub uuid: VaultUuid,
}
impl SharedBroker {
    pub(super) fn new(workbench: Arc<CanonicalWorkbench>) -> Result<Self, BrokerError> {
        validate_owner_roots(&workbench)?;
        let uuid = workbench
            .core
            .owner
            .lock()
            .map_err(|_| BrokerError::WriterBusy)?
            .as_ref()
            .ok_or(BrokerError::WriterBusy)?
            .vault_uuid();
        Ok(Self { workbench, uuid })
    }

    pub(super) fn access<T>(
        &self,
        action: impl FnOnce(&mut FsVaultBroker) -> Result<T, BrokerError>,
    ) -> Result<T, BrokerError> {
        // Canonical reads/writes use current Broker bytes. A derived UI index rebuild must
        // not make a successful batch's next projection fail spuriously.
        validate_owner_roots(&self.workbench)?;
        let mut owner = self
            .workbench
            .core
            .owner
            .lock()
            .map_err(|_| BrokerError::WriterBusy)?;
        let broker = owner.as_mut().ok_or(BrokerError::WriterBusy)?;
        broker.validate_root_identities(
            self.workbench.core.roots.vault_identity(),
            self.workbench.core.roots.writer_data_identity(),
        )?;
        action(broker)
    }
}
fn validate_owner_roots(workbench: &CanonicalWorkbench) -> Result<(), BrokerError> {
    if workbench
        .core
        .control
        .lock()
        .map_err(|_| BrokerError::WriterBusy)?
        .closing
    {
        return Err(BrokerError::WriterBusy);
    }
    workbench
        .core
        .roots
        .revalidate()
        .map_err(|_| BrokerError::WriterBusy)
}
impl VaultReader for SharedBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.uuid
    }
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.access(|b| b.read(path))
    }
}
impl VaultSnapshotter for SharedBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.access(|b| b.snapshot(inputs))
    }
}
impl SnapshotReader for SharedBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.access(|b| b.load_snapshot(id))
    }
}
impl VaultWriter for SharedBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        self.access(|b| b.mutate(mutation))
    }
    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        self.access(|b| b.commit_attempt_artifacts(commit))
    }
}
impl ArtifactStager for SharedBroker {
    fn stage_artifact_commit(
        &self,
        plan: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError> {
        self.access(|b| b.stage_artifact_commit(plan))
    }
    fn load_staged_artifact_commit(
        &self,
        pointer: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError> {
        self.access(|b| b.load_staged_artifact_commit(pointer))
    }
}
impl NumberingScanner for SharedBroker {
    fn scan_number_max(&self, scope: &VaultPath, prefix: &str) -> Result<u64, BrokerError> {
        self.access(|b| b.scan_number_max(scope, prefix))
    }
}
impl BatchWriter for SharedBroker {
    fn preview_batch(&self, request: &BatchRequest) -> Result<BatchPreview, BrokerError> {
        self.access(|b| b.preview_batch(request))
    }
    fn commit_batch(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
    ) -> Result<BatchReceipt, BrokerError> {
        self.access(|b| b.commit_batch(request, preview))
    }
    fn rollback_batch(&mut self, id: &str) -> Result<BatchReceipt, BrokerError> {
        self.access(|b| b.rollback_batch(id))
    }
}
