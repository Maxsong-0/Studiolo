//! `VaultWriter` 的文件系统实现：单对象变更转交 `mutation`，尝试产物逐个仅创建发布、manifest 最后写入。

use std::collections::BTreeSet;

use studiolo_core::{Revision, VaultPath};

use crate::{
    ArtifactCommit, ArtifactCommitReceipt, BrokerError, CommittedArtifact, FsVaultBroker, Mutation,
    MutationReceipt, VaultWriter,
    broker::{verify_artifact_records, verify_manifest},
    mutation, sha256,
    types::AttemptManifest,
};

impl VaultWriter for FsVaultBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        mutation::apply(self, mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        mut commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        if self.write_status()?.unresolved {
            return Err(BrokerError::MutationUnresolved);
        }
        if commit.audit_intent.is_empty() || commit.idempotency_key.is_empty() {
            return Err(BrokerError::InvalidArtifact(
                "audit intent and idempotency key are required".to_owned(),
            ));
        }
        commit
            .artifacts
            .sort_by(|left, right| left.name.cmp(&right.name));
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
        let request_fingerprint = Self::fingerprint(&commit)?;
        let manifest_path = Self::manifest_path(commit.task_id, commit.attempt_id)?;
        if let Some(existing) = self.inspect_existing(&manifest_path)? {
            let manifest: AttemptManifest = serde_json::from_slice(&existing.content)
                .map_err(|error| BrokerError::json("parse attempt manifest", error))?;
            if manifest.idempotency_key == commit.idempotency_key
                && manifest.request_fingerprint == request_fingerprint
            {
                verify_manifest(self, &commit, &manifest, &existing)?;
                return Ok(ArtifactCommitReceipt {
                    manifest_path,
                    manifest_revision: existing.revision,
                    artifacts: manifest.artifacts,
                });
            }
            return Err(BrokerError::IdempotencyConflict);
        }

        let base = format!("sources/attempts/{}/{}", commit.task_id, commit.attempt_id);
        let mut artifacts = Vec::with_capacity(commit.artifacts.len());
        for candidate in &commit.artifacts {
            let target = VaultPath::parse(format!("{base}/{}", candidate.name.as_str()))
                .map_err(|_| BrokerError::InvalidArtifact("invalid derived target".to_owned()))?;
            let expected = Revision::compute(self.vault_uuid, &target, &candidate.content);
            let revision = match self.inspect_existing(&target)? {
                Some(existing) if existing.revision == expected => existing.revision,
                Some(_) => return Err(BrokerError::ArtifactConflict(target)),
                None => {
                    self.mutate(Mutation::create(
                        target.clone(),
                        candidate.content.clone(),
                        format!("artifact:{}:{}", commit.idempotency_key, candidate.name),
                    ))?
                    .revision
                }
            };
            artifacts.push(CommittedArtifact {
                path: target,
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
        let receipt = self.mutate(Mutation::create(
            manifest_path,
            bytes,
            format!("manifest:{}", commit.idempotency_key),
        ))?;
        Ok(ArtifactCommitReceipt {
            manifest_path: receipt.path,
            manifest_revision: receipt.revision,
            artifacts,
        })
    }
}
