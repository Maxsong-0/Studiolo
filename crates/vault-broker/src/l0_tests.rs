//! 本地私有故障契约，不声称等价于断电、真实设备或远端 Audit 可用性验收。

mod compatibility;
mod history;
mod lifecycle;
mod restore;
mod restore_review;
#[path = "../tests/l0_private/restore_write_gate/mod.rs"]
mod restore_write_gate;
mod retention;

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
            .prefix("studiolo-t046-l0-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("data");
        Self {
            _temp: temp,
            root,
            data,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn note(&self) -> Vec<u8> {
        fs::read(self.root.join("note.md")).unwrap()
    }
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn actor() -> MutationActor {
    MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "human".into(),
    }
}
fn metadata() -> MutationMetadata {
    MutationMetadata::new(actor(), MutationKind::Replace).unwrap()
}
fn request(content: &[u8], key: &str) -> Mutation {
    Mutation::create(path(), content.to_vec(), key)
}
fn interrupt(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
fn ack(broker: &mut FsVaultBroker) {
    for _ in 0..10 {
        let batch = broker.pending_content_audit(64).unwrap();
        if batch.is_empty() {
            return;
        }
        for candidate in batch {
            let offset = if candidate.event().phase == ContentAuditPhase::Intent {
                1
            } else {
                2
            };
            broker
                .confirm_content_audit(candidate, offset, &"a".repeat(64))
                .unwrap();
        }
    }
    panic!("audit lifecycle did not drain");
}
fn versions(broker: &mut FsVaultBroker) -> (Revision, Revision) {
    let first = broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap()
        .revision;
    let second = broker
        .mutate_recorded(
            Mutation::replace(path(), b"B".to_vec(), first, "second"),
            metadata(),
        )
        .unwrap()
        .revision;
    (first, second)
}
fn restore_request(
    source: Revision,
    expected: Revision,
    key: &str,
) -> (Mutation, MutationMetadata) {
    (
        Mutation::replace(path(), b"A".to_vec(), expected, key),
        MutationMetadata::new(
            actor(),
            MutationKind::Restore {
                source_revision: source,
            },
        )
        .unwrap(),
    )
}
