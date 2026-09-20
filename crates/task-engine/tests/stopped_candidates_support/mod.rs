//! Shared real candidate fixtures for cancellation recovery contracts.
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_control_db::ControlDb;
use studiolo_core::{TaskId, VaultPath, VaultUuid};
use studiolo_runner_adapter::{PatchMetadata, RunnerArtifact, RunnerReport};
use studiolo_task_engine::*;
use studiolo_vault_broker::*;
pub const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
pub fn spec() -> BoardTaskSpec {
    BoardTaskSpec {
        task: TaskSpec {
            role: "implementer".into(),
            adapter: "claude".into(),
            input_paths: vec![],
            allow_web: vec![],
            allow_db: vec![],
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff_seconds: 0,
                retry_on: vec![],
            },
            timeout: 300,
            project: "review".into(),
            parent: None,
            title: None,
            instruction: None,
        },
        acceptance_criteria: vec!["owner verified the change".into()],
        files_owned: vec![path("src")],
        max_children: 8,
        artifact_target: Some("one implementation".into()),
    }
}
pub fn report(claim: &BoardClaim, patch: &[u8]) -> RunnerReport {
    let metadata = PatchMetadata {
        base_tree: "a".repeat(40),
        after_tree: "b".repeat(40),
        sha256: Sha256::digest(patch)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        retained_path: "/host-retained/candidate.patch".into(),
    };
    RunnerReport {
        task_id: claim.task.task.task_id,
        attempt_id: claim.attempt.attempt_id,
        fence: claim.attempt.fence,
        artifacts: vec![
            RunnerArtifact {
                name: path("candidate.patch"),
                content: patch.to_vec(),
            },
            RunnerArtifact {
                name: path("patch.json"),
                content: serde_json::to_vec(&metadata).unwrap(),
            },
        ],
    }
}
pub fn claimed() -> (
    PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>,
    TaskId,
    BoardClaim,
) {
    let broker = MemoryVaultBroker::new();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(broker.vault_uuid()).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_board_task("runner", spec(), HASH, 0).unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    (engine, task, claim)
}

pub struct LostManifestAck {
    pub inner: FsVaultBroker,
    pub fail: bool,
}
impl VaultReader for LostManifestAck {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, p: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(p)
    }
}
impl VaultSnapshotter for LostManifestAck {
    fn snapshot(&self, i: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(i)
    }
}
impl SnapshotReader for LostManifestAck {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}
impl ArtifactStager for LostManifestAck {
    fn stage_artifact_commit(
        &self,
        p: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError> {
        self.inner.stage_artifact_commit(p)
    }
    fn load_staged_artifact_commit(
        &self,
        p: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError> {
        self.inner.load_staged_artifact_commit(p)
    }
}
impl VaultWriter for LostManifestAck {
    fn mutate(&mut self, m: Mutation) -> Result<MutationReceipt, BrokerError> {
        self.inner.mutate(m)
    }
    fn commit_attempt_artifacts(
        &mut self,
        c: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        let receipt = self.inner.commit_attempt_artifacts(c)?;
        if std::mem::take(&mut self.fail) {
            Err(BrokerError::Io {
                operation: "lost manifest acknowledgement",
                source: std::io::Error::other("fixture crash"),
            })
        } else {
            Ok(receipt)
        }
    }
}

pub fn stopped() -> BoardReceipt {
    BoardReceipt {
        status: ReceiptStatus::Stopped,
        artifacts: vec![],
        conflicts: vec![],
        unlanded: vec![],
        acceptance: vec![AcceptanceAssessment {
            criterion: "owner verified the change".into(),
            result: AssessmentResult::Unverified,
            evidence: vec![],
        }],
        remaining: vec!["owner review has not happened".into()],
        elapsed_ms: 10,
        tool_calls: 2,
        learned: vec![],
        closeout: Closeout {
            learned: vec![],
            adr_required: false,
            rule_change_required: false,
            destinations: vec![],
        },
    }
}
