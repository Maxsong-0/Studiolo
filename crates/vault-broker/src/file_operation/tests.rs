//! 真实临时文件与私有一次性故障点；未运行的契约不是验收结果。
mod failures;
mod large;
mod lifecycle;
mod maintenance;
mod maintenance_crashes;
#[path = "../../tests/file_operation_public_read/mod.rs"]
mod public_read;
#[path = "../../tests/file_rename_private/mod.rs"]
mod rename;
mod stage_evidence;
mod storage;
mod tickets;
use super::*;
use crate::*;
use std::{fs, path::PathBuf, sync::atomic::Ordering};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-file-operations-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn seed(&self, body: &[u8]) -> (FsVaultBroker, Revision) {
        let mut broker = self.open();
        fs::write(self.root.join("note.md"), body).unwrap();
        let revision = Revision::compute(broker.vault_uuid, &path(), body);
        broker.enable_file_operations(limits()).unwrap();
        (broker, revision)
    }
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn limits() -> FileOperationLimits {
    FileOperationLimits {
        effective_object_limit: crate::blob::OBJECT_LIMIT,
        max_tombstones: 64,
        retained_bytes_limit: 8 * 1024 * 1024 * 1024,
        minimum_free_bytes: 1024 * 1024,
    }
}
fn metadata() -> FileOperationMetadata {
    FileOperationMetadata::new(
        MutationActor::Human {
            issuer: "https://issuer.test".into(),
            subject: "human".into(),
        },
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap()
}
fn delete(revision: Revision, key: &str) -> FileOperationRequest {
    FileOperationRequest::Delete {
        path: path(),
        expected: revision,
        idempotency_key: key.into(),
    }
}
fn restore(id: &str, key: &str) -> FileOperationRequest {
    FileOperationRequest::RestoreTombstone {
        path: path(),
        tombstone_id: id.into(),
        idempotency_key: key.into(),
    }
}
fn intent(broker: &mut FsVaultBroker, ticket: &FileOperationTicket) {
    broker
        .confirm_file_operation_intent(ticket, 1, &"a".repeat(64))
        .unwrap();
}
fn flush(broker: &mut FsVaultBroker) {
    for _ in 0..2 {
        for candidate in broker.pending_file_operation_audit(64).unwrap() {
            let offset = if candidate.event().phase == ContentAuditPhase::Intent {
                1
            } else {
                2
            };
            broker
                .confirm_file_operation_audit(candidate, offset, &"a".repeat(64))
                .unwrap();
        }
    }
}
fn committed(broker: &mut FsVaultBroker, request: FileOperationRequest) -> FileOperationReceipt {
    let ticket = broker.prepare_file_operation(request, metadata()).unwrap();
    intent(broker, &ticket);
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Committed);
    flush(broker);
    receipt
}
fn fault(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
