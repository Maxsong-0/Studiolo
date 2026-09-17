//! 经过实际公开 trait 的门禁；私有故障夹具只用于固定协议中断位置。
use super::*;
mod issued;

fn inputs(revision: Revision) -> [SnapshotInput; 1] {
    [SnapshotInput {
        path: path(),
        expected_revision: revision,
    }]
}

// 除最终文件外也核对目录时间，避免拒绝操作先创建再清除快照临时项。
#[derive(Debug, PartialEq, Eq)]
struct SnapshotState {
    entries: Vec<(PathBuf, Vec<u8>)>,
    modified: std::time::SystemTime,
}

fn snapshot_state(fixture: &Fixture) -> SnapshotState {
    let directory = fixture.data.join("snapshots");
    let mut entries: Vec<(PathBuf, Vec<u8>)> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name().into(), fs::read(entry.path()).unwrap())
        })
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    SnapshotState {
        entries,
        modified: fs::metadata(directory).unwrap().modified().unwrap(),
    }
}

fn denied(broker: &FsVaultBroker, fixture: &Fixture, revision: Revision) {
    let before = snapshot_state(fixture);
    let reader: &dyn VaultReader = broker;
    let snapshots: &dyn VaultSnapshotter = broker;
    assert!(matches!(
        reader.read(&path()),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        snapshots.snapshot(&inputs(revision)),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        snapshots.snapshot(&[]),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(snapshot_state(fixture), before);
}

fn readable(broker: &FsVaultBroker, revision: Revision) {
    let reader: &dyn VaultReader = broker;
    let snapshots: &dyn VaultSnapshotter = broker;
    assert_eq!(reader.read(&path()).unwrap().content, b"old");
    let snapshot = snapshots.snapshot(&inputs(revision)).unwrap();
    assert_eq!(snapshot.files.len(), 1);
    assert_eq!(snapshot.files[0].content, b"old");
    assert_eq!(broker.load_snapshot(&snapshot.id).unwrap(), snapshot);
}

#[test]
fn ready_and_typed_prepared_keep_public_current_read_and_snapshot() {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(b"old");
    readable(&broker, revision);
    let ticket = broker
        .prepare_file_operation(delete(revision, "prepared"), metadata())
        .unwrap();
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Prepared);
    readable(&broker, revision);
    let snapshots: &dyn VaultSnapshotter = &broker;
    assert!(snapshots.snapshot(&[]).unwrap().files.is_empty());
    intent(&mut broker, &ticket);
    readable(&broker, revision);
    broker.cancel_file_operation(ticket).unwrap();
    denied(&broker, &fixture, revision);
    flush(&mut broker);
    readable(&broker, revision);
}

#[test]
fn uncertain_writer_cannot_read_or_publish_even_an_empty_snapshot() {
    let fixture = Fixture::new();
    let (broker, revision) = fixture.seed(b"old");
    // 仅私有夹具设置与持久写失败相同的保险状态，不增添公开故障入口。
    broker.writer.poison();
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Uncertain);
    denied(&broker, &fixture, revision);
    assert!(broker.file_operation_status().unwrap().is_none());
}
