//! 私有故障夹具定位阶段中断，不把它描述成断电或磁盘固件持久性证明。

mod prepared;
mod stages;

use super::{load, record::Phase};
use crate::{BrokerError, FsVaultBroker, Mutation, VaultPath, VaultReader, VaultWriter};
use std::{fs, path::PathBuf, sync::atomic::Ordering};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn initialize(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn stage(&self, broker: &FsVaultBroker) -> PathBuf {
        self.root
            .join(".workbench/staging")
            .join(load(broker).unwrap().operation.unwrap().stage.name)
    }
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn request() -> Mutation {
    Mutation::create(path("note.md"), b"intended".to_vec(), "key")
}
fn interrupt(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}

fn assert_blocked(broker: &mut FsVaultBroker) {
    assert!(matches!(
        broker.mutate(Mutation::create(
            path("other.md"),
            b"other".to_vec(),
            "other"
        )),
        Err(BrokerError::MutationUnresolved)
    ));
    let commit = crate::ArtifactCommit {
        task_id: crate::TaskId::new(),
        attempt_id: crate::AttemptId::new(),
        fence: crate::Fence::new(1).unwrap(),
        audit_intent: "fixture".into(),
        idempotency_key: "artifacts".into(),
        artifacts: vec![crate::ArtifactCandidate {
            name: path("out.md"),
            content: b"artifact".to_vec(),
        }],
    };
    assert!(matches!(
        broker.commit_attempt_artifacts(commit),
        Err(BrokerError::MutationUnresolved)
    ));
}

#[test]
fn installing_never_borrows_external_bytes_or_a_missing_stage_as_proof() {
    for remove_stage in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        interrupt(&broker, 2);
        assert!(broker.mutate(request()).is_err());
        assert_eq!(
            load(&broker).unwrap().operation.as_ref().unwrap().phase,
            Phase::Installing
        );
        assert!(!fixture.root.join("note.md").exists());
        let stage = fixture.stage(&broker);
        fs::write(fixture.root.join("note.md"), b"intended").unwrap();
        if remove_stage {
            fs::remove_file(stage).unwrap();
        }
        assert_blocked(&mut broker);
        drop(broker);
        let mut reopened = fixture.open();
        assert_eq!(reopened.write_status().unwrap().committed_through, 0);
        assert!(reopened.write_status().unwrap().unresolved);
        assert!(matches!(
            reopened.mutate(request()),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_blocked(&mut reopened);
        assert_eq!(
            reopened.read(&path("note.md")).unwrap().content,
            b"intended"
        );
    }
}

#[test]
fn installed_same_inode_without_completed_record_is_still_unknown() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    interrupt(&broker, 3);
    assert!(broker.mutate(request()).is_err());
    let operation = load(&broker).unwrap().operation.unwrap();
    let (_, current) = crate::writer::disk::read_identified(&broker.root, "note.md", 1024)
        .unwrap()
        .unwrap();
    assert_eq!(current, operation.stage.identity);
    assert!(!fixture.stage(&broker).exists());
    drop(broker);
    let mut reopened = fixture.open();
    assert!(matches!(
        reopened.mutate(request()),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_blocked(&mut reopened);
    assert_eq!(reopened.write_status().unwrap().committed_through, 0);
}

#[test]
fn completed_only_publishes_receipt_and_watermark_without_reinstalling() {
    for point in [4, 5] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        interrupt(&broker, point);
        assert!(broker.mutate(request()).is_err());
        assert_eq!(
            load(&broker).unwrap().operation.unwrap().phase,
            Phase::Completed
        );
        fs::write(fixture.root.join("note.md"), b"later-external-content").unwrap();
        drop(broker);
        let mut reopened = fixture.open();
        let status = reopened.write_status().unwrap();
        assert!(!status.unresolved);
        assert_eq!(status.committed_through, 1);
        let receipt = reopened.mutate(request()).unwrap();
        assert_eq!(
            receipt.revision,
            crate::Revision::compute(reopened.vault_uuid(), &path("note.md"), b"intended")
        );
        assert_eq!(
            reopened.read(&path("note.md")).unwrap().content,
            b"later-external-content"
        );
        assert_eq!(reopened.write_status().unwrap().committed_through, 1);
        drop(reopened);
        assert_eq!(fixture.open().write_status().unwrap().committed_through, 1);
    }
}
