//! 实际临时文件系统和私有中断点；不冒充断电或管理员权限验收。

mod installing;
mod precondition;
mod prepared;
mod prune;
mod review;
mod stream_compat;
mod stream_crashes;
mod stream_support;

use crate::{
    BrokerError, FsVaultBroker, Mutation, PendingDecision, ReconciliationOutcome, ScanLimits,
    VaultPath, VaultReader, VaultWriter, WriterPhase,
};
use std::{fs, path::PathBuf, sync::atomic::Ordering};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-")
            .tempdir_in(parent)
            .unwrap();
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
    fn interrupted(&self, point: u8) -> (FsVaultBroker, Mutation) {
        let mut broker = self.initialize();
        let first = broker
            .mutate(Mutation::create(path(), b"old".to_vec(), "initial"))
            .unwrap();
        let request = Mutation::replace(path(), b"proposed".to_vec(), first.revision, "pending");
        interrupt(&broker, point);
        assert!(matches!(
            broker.mutate(request.clone()),
            Err(BrokerError::Io {
                operation: "injected writer interruption",
                ..
            })
        ));
        (broker, request)
    }
    fn stage(&self, broker: &FsVaultBroker) -> PathBuf {
        self.root.join(".workbench/staging").join(
            crate::mutation::load(broker)
                .unwrap()
                .operation
                .unwrap()
                .stage
                .name,
        )
    }
    fn evidence(&self, id: &str, role: &str) -> Vec<u8> {
        fs::read(
            self.data
                .join("snapshots")
                .join(format!("recovery-{id}-{role}.bin")),
        )
        .unwrap()
    }
}

fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn interrupt(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
fn preserve() -> PendingDecision {
    PendingDecision::PreserveCurrent {
        original_content: b"proposed".to_vec(),
    }
}
fn assert_ready(broker: &FsVaultBroker) {
    let status = broker.write_status().unwrap();
    assert_eq!(status.phase, WriterPhase::Ready);
    assert!(!status.unresolved);
    assert!(!status.read_blocked);
}
