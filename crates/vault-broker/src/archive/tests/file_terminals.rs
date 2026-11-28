//! 删除、恢复、取消、拒绝的文件操作也能打包（T-249 G3）；终态未知的留散件。
//! 打包前后：最近删除、恢复、历史、墓碑名额、保留字节、保护的 revision 一样，旧键照答原回执。
use super::renames::{actor, enable, rename, run};
use super::*;
use crate::file_operation::{disk::Folder, tombstone, usage};

/// 做完一次文件操作，回执都落；取消的在意图回执之后取消。交回原回执。
fn finish(
    broker: &mut FsVaultBroker,
    request: FileOperationRequest,
    offset: u64,
    cancel: bool,
) -> FileOperationReceipt {
    let ticket = broker.prepare_file_operation(request, actor()).unwrap();
    broker
        .confirm_file_operation_intent(&ticket, offset, HASH)
        .unwrap();
    let receipt = if cancel {
        broker.cancel_file_operation(ticket).unwrap()
    } else {
        broker.commit_file_operation(&ticket).unwrap()
    };
    for candidate in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(candidate, offset + 1, HASH)
            .unwrap();
    }
    receipt
}
fn delete(broker: &FsVaultBroker, path: &str, key: &str) -> FileOperationRequest {
    FileOperationRequest::Delete {
        path: logical(path),
        expected: broker.read(&logical(path)).unwrap().revision,
        idempotency_key: key.into(),
    }
}

/// 四篇真写的；改名、删除、恢复、拒绝（旧版本对不上）、取消各一次。
fn setup(
    fixture: &Fixture,
) -> (
    FsVaultBroker,
    Vec<(FileOperationRequest, FileOperationReceipt)>,
) {
    let mut broker = fixture.open();
    notes(&mut broker, 4);
    enable(&mut broker);
    let mut done = Vec::new();
    let request = rename(&broker, "note-0.md", "moved.md", "move");
    done.push((request.clone(), run(&mut broker, request, 50)));
    let request = delete(&broker, "note-1.md", "delete");
    let deleted = finish(&mut broker, request.clone(), 60, false);
    assert_eq!(deleted.outcome, FileOperationOutcome::Committed);
    done.push((request, deleted.clone()));
    let request = FileOperationRequest::RestoreTombstone {
        path: logical("note-1.md"),
        tombstone_id: deleted.tombstone_id.unwrap(),
        idempotency_key: "restore".into(),
    };
    let restored = finish(&mut broker, request.clone(), 70, false);
    assert_eq!(restored.outcome, FileOperationOutcome::Committed);
    done.push((request, restored));
    let request = FileOperationRequest::Delete {
        path: logical("note-2.md"),
        expected: broker.read(&logical("note-3.md")).unwrap().revision,
        idempotency_key: "stale".into(),
    };
    let rejected = finish(&mut broker, request.clone(), 80, false);
    assert_eq!(rejected.outcome, FileOperationOutcome::Rejected);
    done.push((request, rejected));
    let request = delete(&broker, "note-3.md", "cancel");
    let cancelled = finish(&mut broker, request.clone(), 90, true);
    assert_eq!(cancelled.outcome, FileOperationOutcome::Cancelled);
    done.push((request, cancelled));
    (broker, done)
}

type Observed = (
    Vec<FileOperationReceipt>,
    Vec<TombstoneEntry>,
    Vec<HistoryPage>,
    Vec<Vec<Revision>>,
    (u64, usize),
);
fn observe(
    broker: &FsVaultBroker,
    done: &[(FileOperationRequest, FileOperationReceipt)],
) -> Observed {
    let paths = [
        "note-0.md",
        "moved.md",
        "note-1.md",
        "note-2.md",
        "note-3.md",
    ]
    .map(logical);
    (
        done.iter()
            .map(|(request, _)| {
                broker
                    .file_operation_replay(request, &actor())
                    .unwrap()
                    .unwrap()
                    .receipt
                    .unwrap()
            })
            .collect(),
        broker.list_tombstones(10, None).unwrap().entries,
        paths
            .iter()
            .map(|path| broker.list_history(path, 10, None).unwrap())
            .collect(),
        paths
            .iter()
            .map(|path| tombstone::protected(broker, path).unwrap())
            .collect(),
        usage::actual(broker).unwrap(),
    )
}

/// 原来只打改名：删除、恢复、拒绝、取消的请求、回执和重放永远留散件。
#[test]
fn settled_file_operations_are_packed_and_change_no_answer_or_budget() {
    let fixture = Fixture::new();
    let (mut broker, done) = setup(&fixture);
    let before = observe(&broker, &done);
    for ((_, receipt), replayed) in done.iter().zip(&before.0) {
        assert_eq!(receipt, replayed);
    }
    let records = all_records(&broker);
    // 四篇正文、五次文件操作。
    assert_eq!(pack(&mut broker, later()), 9);
    assert!(
        file_operation::disk::names(&broker, Folder::Records)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        file_operation::disk::names(&broker, Folder::Tombstones)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(all_records(&broker), records);
    assert_eq!(observe(&broker, &done), before);
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(observe(&broker, &done), before);
    // 删除的墓碑经打了包的删除记录照样能再恢复一次。
    let again = delete(&broker, "note-1.md", "delete-again");
    run(&mut broker, again, 100);
    let restore = FileOperationRequest::RestoreTombstone {
        path: logical("note-1.md"),
        tombstone_id: done[1].1.tombstone_id.clone().unwrap(),
        idempotency_key: "restore-again".into(),
    };
    run(&mut broker, restore, 110);
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    // 四篇正文、七次文件操作，各两份回执。
    assert_eq!(sealed.unwrap(), 2 * 4 + 2 * 7);
    drop(broker);
    let mut broker = fixture.reopen();
    let loose = loose_records(&fixture.data);
    let restored = broker.archive_unpack().unwrap();
    assert_eq!(restored.packs, 9);
    assert_eq!(loose_records(&fixture.data).len(), loose.len() + 9 * 4);
}

/// 重放散件被正文占满时，文件操作也在落盘前被拒（和封存同一把尺，T-249 G1）；打包之后照常，
/// 落定的文件操作跟着进段，旧键照答。真做 6000 次文件操作在用例里是平方级（每次都要读全部文件操作散件算保留），
/// 所以散件用合成的正文垫满。
#[test]
fn file_operations_resume_once_packing_frees_the_replays() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    enable(&mut broker);
    let wrong = broker.read(&logical("note-1.md")).unwrap().revision;
    let stale = |index: usize| FileOperationRequest::Delete {
        path: logical("note-0.md"),
        expected: wrong,
        idempotency_key: format!("stale-{index}"),
    };
    for index in 0..4 {
        let receipt = finish(&mut broker, stale(index), 100 + 2 * index as u64, false);
        assert_eq!(receipt.outcome, FileOperationOutcome::Rejected);
    }
    // 两篇正文、四次文件操作，再垫 4090 条：重放正好 4096 条。
    bulk(&broker, 0..4090);
    assert!(matches!(
        broker.prepare_file_operation(stale(4), actor()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(pack(&mut broker, later()), 4096);
    let receipt = finish(&mut broker, stale(4), 200, false);
    assert_eq!(receipt.outcome, FileOperationOutcome::Rejected);
    for index in 0..4 {
        let answer = broker
            .file_operation_replay(&stale(index), &actor())
            .unwrap()
            .and_then(|status| status.receipt)
            .map(|receipt| receipt.outcome);
        assert_eq!(answer, Some(FileOperationOutcome::Rejected), "{index}");
    }
    let usage = broker.record_usage().unwrap();
    assert_eq!((usage.replays.used, usage.file_operations.used), (1, 1));
}
