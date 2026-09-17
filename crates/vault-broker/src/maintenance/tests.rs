//! 真实临时文件、旧请求和私有中断点；不冒充断电或真实 TTY/Audit 验收。
mod crashes;
mod guards;
mod lifecycle;
mod restore;
mod storage;
mod unsafe_target;
use super::*;
use crate::{Mutation, VaultPath, VaultReader, VaultWriter, WriterPhase};
use std::{fs, path::PathBuf, sync::atomic::Ordering};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-maintenance-")
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
    fn pending(&self, phase: u8) -> (FsVaultBroker, Mutation) {
        let mut broker = self.open();
        let first = broker
            .mutate(Mutation::create(path(), b"old".to_vec(), "first"))
            .unwrap();
        let request = Mutation::replace(path(), b"proposed".to_vec(), first.revision, "pending");
        fault(&broker, phase);
        assert!(broker.mutate(request.clone()).is_err());
        (broker, request)
    }
    fn body(&self) -> Vec<u8> {
        fs::read(self.root.join("note.md")).unwrap()
    }
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn actor() -> MaintenanceActor {
    MaintenanceActor::current().unwrap()
}
fn fault(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
fn prepare(broker: &mut FsVaultBroker, decision: MaintenanceDecision) -> MaintenanceTicket {
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    broker
        .prepare_maintenance(&target.review_id, decision, actor())
        .unwrap()
}
fn intent(broker: &mut FsVaultBroker, ticket: &MaintenanceTicket) {
    broker
        .confirm_maintenance_intent(ticket, 1, &"a".repeat(64))
        .unwrap();
}
fn flush(broker: &mut FsVaultBroker) {
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        let offset = if candidate.event().phase == ContentAuditPhase::Intent {
            1
        } else {
            2
        };
        broker
            .confirm_maintenance_audit(candidate, offset, &"a".repeat(64))
            .unwrap();
    }
}
fn flush_content(broker: &mut FsVaultBroker) {
    for _ in 0..4 {
        for candidate in broker.pending_content_audit(64).unwrap() {
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
}
