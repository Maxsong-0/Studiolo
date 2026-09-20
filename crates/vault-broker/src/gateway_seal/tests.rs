//! 单次调用封存独立于最终产物提交；只使用临时目录与内存替身。
use super::*;
use crate::{
    ArtifactCandidate, ArtifactCommit, BatchChange, BatchEdit, BatchRequest, BatchWriter,
    FsVaultBroker, MemoryVaultBroker, VaultWriter,
};
use std::{fs, sync::atomic::Ordering};
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn seal(call_id: &str) -> GatewayCallSeal {
    GatewayCallSeal {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        call_id: call_id.into(),
        audit_intent: "audit-intent".into(),
        raw_sha256: "a".repeat(64),
        raw_bytes: 120,
        source_manifest_sha256: "b".repeat(64),
        model_provider: "provider".into(),
        model: "model".into(),
        retain_until_unix_seconds: 2_000_000_000,
        snapshot_id: "c".repeat(64),
        effective_policy_hash: "d".repeat(64),
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    vault: std::path::PathBuf,
    data: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            vault: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.vault, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.vault, &self.data).unwrap()
    }
}
fn contract(broker: &mut (impl GatewayPublisher + VaultWriter)) {
    let first = seal("call-one");
    let receipt = broker.seal_gateway_call(first.clone()).unwrap();
    assert_eq!(broker.seal_gateway_call(first.clone()).unwrap(), receipt);
    let mut second = first.clone();
    second.call_id = "call-two".into();
    let second_receipt = broker.seal_gateway_call(second).unwrap();
    assert_ne!(receipt.manifest_path, second_receipt.manifest_path);
    assert!(
        receipt
            .manifest_path
            .as_str()
            .starts_with(".workbench/gateway/")
    );
    let final_receipt = broker
        .commit_attempt_artifacts(ArtifactCommit {
            task_id: first.task_id,
            attempt_id: first.attempt_id,
            fence: first.fence,
            audit_intent: "final-intent".into(),
            idempotency_key: "final-result".into(),
            artifacts: vec![ArtifactCandidate {
                name: path("answer.md"),
                content: b"final answer".to_vec(),
            }],
        })
        .unwrap();
    assert!(
        final_receipt
            .manifest_path
            .as_str()
            .starts_with("sources/attempts/")
    );
    assert_ne!(final_receipt.manifest_path, receipt.manifest_path);
    assert_eq!(broker.seal_gateway_call(first.clone()).unwrap(), receipt);
    let mut changed = first;
    changed.raw_sha256 = "e".repeat(64);
    assert!(matches!(
        broker.seal_gateway_call(changed),
        Err(BrokerError::IdempotencyConflict)
    ));
}
#[test]
fn independent_calls_and_final_artifacts_have_distinct_commit_points() {
    contract(&mut MemoryVaultBroker::new());
    contract(&mut Fixture::new().open());
}
#[test]
fn reboot_replays_exact_metadata_without_l0_or_raw_copy() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let request = seal("restart");
    let receipt = broker.seal_gateway_call(request.clone()).unwrap();
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
    assert_eq!(
        fs::read_dir(fixture.data.join("history")).unwrap().count(),
        0
    );
    let bytes = fs::read(fixture.vault.join(receipt.manifest_path.as_str())).unwrap();
    assert_eq!(crate::sha256(&bytes), receipt.sha256);
    let stored: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["call"]["raw_bytes"], 120);
    assert!(stored["call"].get("raw").is_none());
    assert!(stored["call"].get("content").is_none());
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(broker.seal_gateway_call(request).unwrap(), receipt);
}
#[test]
fn malicious_call_id_is_always_hashed_into_one_safe_component() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let request = seal("../../../../outside/$secret");
    let receipt = broker.seal_gateway_call(request.clone()).unwrap();
    let expected = format!(
        ".workbench/gateway/{}/{}/{}/manifest.json",
        request.task_id,
        request.attempt_id,
        crate::sha256(request.call_id.as_bytes())
    );
    assert_eq!(receipt.manifest_path.as_str(), expected);
    assert!(!receipt.manifest_path.as_str().contains(".."));
    assert!(!fixture._temp.path().join("outside").exists());
    assert!(fixture.vault.join(receipt.manifest_path.as_str()).is_file());
}
#[test]
fn pending_batch_blocks_private_gateway_publication() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let request = BatchRequest {
        batch_id: "pending".into(),
        changes: vec![BatchChange {
            path: path("note.md"),
            edit: BatchEdit::Create {
                content: "body".into(),
            },
        }],
    };
    let preview = broker.preview_batch(&request).unwrap();
    broker.writer.fault.store(42, Ordering::Release);
    assert!(broker.commit_batch(request, &preview).is_err());
    assert!(matches!(
        broker.seal_gateway_call(seal("blocked")),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(!fixture.vault.join(".workbench/gateway").exists());
}
#[test]
fn invalid_identity_hash_and_retention_are_rejected_without_writes() {
    let mut broker = MemoryVaultBroker::new();
    let mut request = seal("valid");
    request.raw_sha256 = "not-a-digest".into();
    assert!(broker.seal_gateway_call(request).is_err());
    let mut request = seal("bad\ncall");
    request.retain_until_unix_seconds = 0;
    assert!(broker.seal_gateway_call(request).is_err());
    assert!(broker.gateway_seals.is_empty());
}
#[cfg(unix)]
#[test]
fn linked_private_namespace_cannot_redirect_a_manifest() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let outside = fixture._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.vault.join(".workbench/gateway")).unwrap();
    assert!(broker.seal_gateway_call(seal("symlink")).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
