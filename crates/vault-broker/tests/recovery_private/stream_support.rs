//! 旧 Vec 安装未决与大 current 的真实临时文件兼容；不代表断电/Audit 验收。
use super::*;
use crate::{ContentAuditPhase, MaintenanceActor};
use std::io::Write;

pub(super) const LARGE: usize = crate::writer::disk::CONTENT_LIMIT + 1;
pub(super) fn old_pending(
    f: &Fixture,
    size: usize,
) -> (
    FsVaultBroker,
    Mutation,
    crate::mutation::record::Operation,
    Vec<u8>,
) {
    let (broker, request) = f.interrupted(2);
    let state = crate::mutation::load(&broker).unwrap();
    assert_eq!(state.format_version, 2);
    let op = state.operation.unwrap();
    let original = envelope(&broker, &op);
    drop(broker);
    // 实际写入固定缓冲，不用稀疏文件伪造流式内容验证。
    let mut file = fs::File::create(f.root.join("note.md")).unwrap();
    let block = [b'e'; 64 * 1024];
    let mut left = size;
    while left > 0 {
        let n = left.min(block.len());
        file.write_all(&block[..n]).unwrap();
        left -= n;
    }
    file.sync_all().unwrap();
    drop(file);
    let broker = f.open();
    assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 2);
    assert_eq!(
        broker.write_status().unwrap().phase,
        WriterPhase::Installing
    );
    (broker, request, op, original)
}
pub(super) fn envelope(broker: &FsVaultBroker, op: &crate::mutation::record::Operation) -> Vec<u8> {
    serde_json::to_vec(
        &crate::recorded::store::load(broker, op.recording.as_ref().unwrap()).unwrap(),
    )
    .unwrap()
}
pub(super) fn assert_package(
    f: &Fixture,
    broker: &FsVaultBroker,
    id: &str,
    op: &crate::mutation::record::Operation,
    size: usize,
    version: u32,
) {
    let bytes = fs::read(f.data.join("snapshots").join(format!("recovery-{id}.json"))).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(manifest["version"], version);
    assert_eq!(manifest["fingerprint"], op.fingerprint);
    assert_eq!(f.evidence(id, "proposed"), b"proposed");
    assert_eq!(f.evidence(id, "previous"), b"old");
    assert_eq!(
        fs::metadata(
            f.data
                .join("snapshots")
                .join(format!("recovery-{id}-current.bin"))
        )
        .unwrap()
        .len(),
        size as u64
    );
    let current = crate::blob::file::scan(
        &broker.root,
        &path(),
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &path())),
    )
    .unwrap();
    assert_eq!(current.identity.bytes, size as u64);
    assert_eq!(
        super::super::evidence::verify_version(broker, id, op, current.revision).unwrap(),
        version
    );
}
pub(super) fn actor() -> MaintenanceActor {
    MaintenanceActor::current().unwrap()
}
pub(super) fn ack_maintenance(broker: &mut FsVaultBroker) {
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
