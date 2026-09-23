//! 内存契约适配器：与文件系统适配器遵循相同的逻辑 CAS、快照与 manifest 规则。

use std::{collections::BTreeSet, sync::Mutex};

use studiolo_core::{Revision, VaultPath, VaultUuid};

use crate::{
    ArtifactCommit, ArtifactCommitReceipt, BrokerError, CommittedArtifact, FsVaultBroker, Mutation,
    MutationReceipt, Snapshot, SnapshotFile, SnapshotId, SnapshotInput, SnapshotReader,
    VaultObject, VaultReader, VaultSnapshotter, VaultWriter, batch,
    broker::{verify_artifact_records, verify_manifest},
    memory_mutation, memory_namespace, memory_state_error, mutation, sha256,
    types::{AttemptManifest, snapshot_identity},
};

/// 内存契约适配器，遵循相同的逻辑 CAS 与 manifest 规则。
pub struct MemoryVaultBroker {
    pub(crate) vault_uuid: VaultUuid,
    pub(crate) objects: std::collections::BTreeMap<VaultPath, Vec<u8>>,
    pub(crate) history: std::collections::BTreeMap<Revision, Vec<u8>>,
    snapshots: Mutex<std::collections::BTreeMap<SnapshotId, Snapshot>>,
    pub(crate) replays: std::collections::BTreeMap<String, (String, mutation::record::Outcome)>,
    pub(crate) artifact_stages: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    pub(crate) batch_replays: std::collections::BTreeMap<String, batch::memory::MemoryBatch>,
    pub(crate) gateway_seals: std::collections::BTreeMap<VaultPath, Vec<u8>>,
}

impl Default for MemoryVaultBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryVaultBroker {
    /// 用新生成的 Vault 身份创建空适配器。
    #[must_use]
    pub fn new() -> Self {
        Self::with_uuid(VaultUuid::new())
    }

    /// 用指定的 Vault 持久身份创建空适配器。
    #[must_use]
    pub fn with_uuid(vault_uuid: VaultUuid) -> Self {
        Self {
            vault_uuid,
            objects: std::collections::BTreeMap::new(),
            history: std::collections::BTreeMap::new(),
            snapshots: Mutex::new(std::collections::BTreeMap::new()),
            replays: std::collections::BTreeMap::new(),
            artifact_stages: Mutex::new(std::collections::BTreeMap::new()),
            batch_replays: std::collections::BTreeMap::new(),
            gateway_seals: std::collections::BTreeMap::new(),
        }
    }

    /// 返回 L0 历史对象，供契约断言使用。
    #[must_use]
    pub fn history_object(&self, revision: Revision) -> Option<&[u8]> {
        self.history.get(&revision).map(Vec::as_slice)
    }

    fn memory_snapshot_id(&self, files: &[SnapshotFile]) -> Result<SnapshotId, BrokerError> {
        snapshot_identity(self.vault_uuid, files)
    }

    /// 与 `FsVaultBroker::inspect_existing` 同形：缺失是 `None`，其余失败一律上抛，
    /// 不把 `PathCollision` / `UnsafeNode` 静默当成「还不存在」。
    pub(crate) fn inspect_existing(
        &self,
        path: &VaultPath,
    ) -> Result<Option<VaultObject>, BrokerError> {
        match self.read(path) {
            Ok(object) => Ok(Some(object)),
            Err(BrokerError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl VaultReader for MemoryVaultBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.vault_uuid
    }

    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        memory_namespace::guard_read(self, path)?;
        let content = if path.as_str() == memory_namespace::IDENTITY_MARKER {
            memory_namespace::identity_bytes(self.vault_uuid)?
        } else {
            self.objects
                .get(path)
                .cloned()
                .ok_or_else(|| BrokerError::NotFound(path.clone()))?
        };
        Ok(VaultObject {
            revision: Revision::compute(self.vault_uuid, path, &content),
            path: path.clone(),
            content,
        })
    }
}

impl VaultSnapshotter for MemoryVaultBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        let mut inputs = inputs.to_vec();
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        if inputs.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(BrokerError::InvalidArtifact(
                "snapshot contains duplicate paths".to_owned(),
            ));
        }
        let mut files = Vec::with_capacity(inputs.len());
        for input in inputs {
            let object = self.read(&input.path)?;
            if object.revision != input.expected_revision {
                return Err(BrokerError::RevisionConflict {
                    path: input.path,
                    expected: input.expected_revision.to_string(),
                    actual: Some(object.revision),
                });
            }
            files.push(SnapshotFile {
                path: object.path,
                revision: object.revision,
                content: object.content,
            });
        }
        let snapshot = Snapshot {
            id: self.memory_snapshot_id(&files)?,
            vault_uuid: self.vault_uuid,
            files,
        };
        self.snapshots
            .lock()
            .map_err(|_| memory_state_error())?
            .insert(snapshot.id.clone(), snapshot.clone());
        Ok(snapshot)
    }
}

impl SnapshotReader for MemoryVaultBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.snapshots
            .lock()
            .map_err(|_| memory_state_error())?
            .get(id)
            .cloned()
            .ok_or_else(|| {
                BrokerError::io(
                    "load memory snapshot",
                    std::io::Error::from(std::io::ErrorKind::NotFound),
                )
            })
    }
}

impl VaultWriter for MemoryVaultBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        memory_mutation::apply(self, mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        mut commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        if commit.audit_intent.is_empty() || commit.idempotency_key.is_empty() {
            return Err(BrokerError::InvalidArtifact(
                "audit intent and idempotency key are required".to_owned(),
            ));
        }
        commit
            .artifacts
            .sort_by(|left, right| left.name.cmp(&right.name));
        // 与磁盘同序：保留名与撞名必须在任何产物落盘之前判完，被拒绝的请求零副作用。
        let mut seen = BTreeSet::new();
        for artifact in &commit.artifacts {
            if artifact.name.as_str() == "manifest.json" || artifact.name.is_protected() {
                return Err(BrokerError::InvalidArtifact(format!(
                    "reserved artifact name: {}",
                    artifact.name
                )));
            }
            if !seen.insert(artifact.name.collision_key()) {
                return Err(BrokerError::InvalidArtifact(
                    "artifact names collide".to_owned(),
                ));
            }
        }
        let request_fingerprint = FsVaultBroker::fingerprint(&commit)?;
        let manifest_path = FsVaultBroker::manifest_path(commit.task_id, commit.attempt_id)?;
        if let Some(existing) = self.inspect_existing(&manifest_path)? {
            let manifest: AttemptManifest = serde_json::from_slice(&existing.content)
                .map_err(|error| BrokerError::json("parse attempt manifest", error))?;
            if manifest.idempotency_key != commit.idempotency_key
                || manifest.request_fingerprint != request_fingerprint
            {
                return Err(BrokerError::IdempotencyConflict);
            }
            verify_manifest(self, &commit, &manifest, &existing)?;
            return Ok(ArtifactCommitReceipt {
                manifest_path,
                manifest_revision: existing.revision,
                artifacts: manifest.artifacts,
            });
        }
        let base = format!("sources/attempts/{}/{}", commit.task_id, commit.attempt_id);
        let mut artifacts = Vec::new();
        for candidate in &commit.artifacts {
            let path = VaultPath::parse(format!("{base}/{}", candidate.name.as_str()))
                .map_err(|_| BrokerError::InvalidArtifact("invalid derived target".to_owned()))?;
            let expected = Revision::compute(self.vault_uuid, &path, &candidate.content);
            let revision = match self.inspect_existing(&path)? {
                Some(object) if object.revision == expected => object.revision,
                Some(_) => return Err(BrokerError::ArtifactConflict(path)),
                None => {
                    self.mutate(Mutation::create(
                        path.clone(),
                        candidate.content.clone(),
                        format!("artifact:{}:{}", commit.idempotency_key, candidate.name),
                    ))?
                    .revision
                }
            };
            artifacts.push(CommittedArtifact {
                path,
                revision,
                sha256: sha256(&candidate.content),
            });
        }
        let manifest = AttemptManifest {
            format_version: 1,
            task_id: commit.task_id.to_string(),
            attempt_id: commit.attempt_id.to_string(),
            fence: commit.fence.get(),
            audit_intent: commit.audit_intent,
            idempotency_key: commit.idempotency_key.clone(),
            request_fingerprint,
            artifacts: artifacts.clone(),
        };
        verify_artifact_records(self, &artifacts)?;
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| BrokerError::json("serialize attempt manifest", error))?;
        let manifest_receipt = self.mutate(Mutation::create(
            manifest_path,
            bytes,
            format!("manifest:{}", commit.idempotency_key),
        ))?;
        Ok(ArtifactCommitReceipt {
            manifest_path: manifest_receipt.path,
            manifest_revision: manifest_receipt.revision,
            artifacts,
        })
    }
}
