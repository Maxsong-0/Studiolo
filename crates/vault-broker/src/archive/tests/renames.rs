use super::*;
use crate::file_operation::{disk::Folder, tombstone, usage};

pub(super) fn actor() -> FileOperationMetadata {
    FileOperationMetadata::new(
        MutationActor::Obsidian,
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap()
}
pub(super) fn run(
    broker: &mut FsVaultBroker,
    request: FileOperationRequest,
    offset: u64,
) -> FileOperationReceipt {
    let ticket = broker.prepare_file_operation(request, actor()).unwrap();
    broker
        .confirm_file_operation_intent(&ticket, offset, HASH)
        .unwrap();
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Committed);
    for candidate in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(candidate, offset + 1, HASH)
            .unwrap();
    }
    receipt
}
pub(super) fn rename(
    broker: &FsVaultBroker,
    from: &str,
    to: &str,
    key: &str,
) -> FileOperationRequest {
    FileOperationRequest::Rename {
        from: logical(from),
        expected: broker.read(&logical(from)).unwrap().revision,
        to: logical(to),
        idempotency_key: key.into(),
    }
}
pub(super) fn enable(broker: &mut FsVaultBroker) {
    broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: crate::blob::OBJECT_LIMIT,
            max_tombstones: 64,
            retained_bytes_limit: 8 * 1024 * 1024 * 1024,
            minimum_free_bytes: 1024 * 1024,
        })
        .unwrap();
    broker.enable_file_rename().unwrap();
}
/// 两篇真写的、一次改名、一次删除；回执都落了。
fn setup(fixture: &Fixture) -> (FsVaultBroker, FileOperationRequest, FileOperationReceipt) {
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    enable(&mut broker);
    let request = rename(&broker, "note-0.md", "moved.md", "move");
    let receipt = run(&mut broker, request.clone(), 50);
    let delete = FileOperationRequest::Delete {
        path: logical("note-1.md"),
        expected: broker.read(&logical("note-1.md")).unwrap().revision,
        idempotency_key: "delete".into(),
    };
    run(&mut broker, delete, 60);
    (broker, request, receipt)
}

/// 打包改名、不打包删除；改名的答复、最近删除、历史、名额、保留字节、保护的 revision 打包前后都一样。
#[test]
fn a_packed_rename_changes_no_answer_and_loosens_no_budget() {
    let fixture = Fixture::new();
    let (mut broker, request, receipt) = setup(&fixture);
    let observe = |broker: &FsVaultBroker| {
        let paths = ["note-0.md", "moved.md", "note-1.md"].map(logical);
        (
            broker
                .file_operation_replay(&request, &actor())
                .unwrap()
                .unwrap()
                .receipt
                .unwrap(),
            broker.list_tombstones(10, None).unwrap().entries,
            paths
                .iter()
                .map(|path| broker.list_history(path, 10, None).unwrap())
                .collect::<Vec<_>>(),
            paths
                .iter()
                .map(|path| tombstone::protected(broker, path).unwrap())
                .collect::<Vec<_>>(),
            usage::actual(broker).unwrap(),
        )
    };
    let before = observe(&broker);
    assert_eq!(before.0, receipt);
    let records = all_records(&broker);
    assert_eq!(pack(&mut broker, later()), 3);
    // 删除的请求、两份回执和重放留着；墓碑两个都在。
    assert_eq!(
        file_operation::disk::names(&broker, Folder::Records)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        file_operation::disk::names(&broker, Folder::Tombstones)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(all_records(&broker), records);
    assert_eq!(observe(&broker), before);
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(observe(&broker), before);
    // 同一个键换个动作还是冲突；别的键照常改名。
    let mut other = request.clone();
    if let FileOperationRequest::Rename { to, .. } = &mut other {
        *to = logical("elsewhere.md");
    }
    assert!(broker.file_operation_replay(&other, &actor()).is_err());
    assert!(matches!(
        broker.mutate_recorded(
            Mutation::create(logical("x.md"), b"x".to_vec(), "move"),
            metadata()
        ),
        Err(BrokerError::IdempotencyConflict)
    ));
    let next = rename(&broker, "moved.md", "again.md", "move-2");
    run(&mut broker, next, 70);
    let (mut broker, sealed) = checkpoint(&fixture, broker, "seal");
    assert_eq!(sealed.unwrap(), 10);
    // 改名的墓碑照样能恢复到原路径。
    let restore = FileOperationRequest::RestoreTombstone {
        path: logical("note-0.md"),
        tombstone_id: receipt.tombstone_id.unwrap(),
        idempotency_key: "bring-back".into(),
    };
    run(&mut broker, restore, 80);
    assert_eq!(
        broker.read(&logical("note-0.md")).unwrap().content,
        "正文 0".as_bytes()
    );
}

#[test]
fn packing_renames_resumes_and_unpacks_like_content() {
    for point in 100..=105 {
        let fixture = Fixture::new();
        let (mut broker, _, _) = setup(&fixture);
        let before = all_records(&broker);
        let loose = loose_records(&fixture.data);
        fault(&broker, point);
        interrupted(&mut broker);
        drop(broker);
        let mut broker = fixture.reopen();
        assert_eq!(all_records(&broker), before, "{point}");
        pack(&mut broker, later());
        assert_eq!(all_records(&broker), before, "{point}");
        let done = broker.archive_unpack().unwrap();
        assert_eq!((done.packs, done.file_operations), (3, 2));
        assert_eq!(loose_records(&fixture.data), loose, "{point}");
    }
}
/// 改名满了年限、但效果落定还没满，不打。
#[test]
fn a_rename_must_also_have_taken_effect_seven_days_ago() {
    let fixture = Fixture::new();
    let (mut broker, _, _) = setup(&fixture);
    let cutoff = ArchiveCutoff {
        sealed_offset: 51,
        ..later()
    };
    // 删除不打、正文两条打、改名的两份回执是 50 和 51。
    assert_eq!(pack(&mut broker, cutoff), 3);
    let fixture = Fixture::new();
    let (mut broker, _, _) = setup(&fixture);
    let cutoff = ArchiveCutoff {
        sealed_offset: 50,
        ..later()
    };
    assert_eq!(pack(&mut broker, cutoff), 2);
}

/// 改名没做成（经维护取消）的不打；效果落定还没满 7 天的也不打，哪怕请求早就满了。
#[test]
fn only_a_committed_rename_whose_effect_is_old_enough_is_packed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    enable(&mut broker);
    let request = rename(&broker, "note-0.md", "never.md", "cancelled");
    let ticket = broker.prepare_file_operation(request, actor()).unwrap();
    broker
        .confirm_file_operation_intent(&ticket, 50, HASH)
        .unwrap();
    let review = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let cancel = broker
        .prepare_maintenance(
            &review.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&cancel, 52, HASH)
        .unwrap();
    broker.commit_maintenance(cancel).unwrap();
    for candidate in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(candidate, 51, HASH)
            .unwrap();
    }
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 53, HASH)
            .unwrap();
    }
    assert_eq!(pack(&mut broker, later()), 2);
    assert_eq!(
        file_operation::disk::names(&broker, Folder::Records)
            .unwrap()
            .len(),
        1
    );

    let fixture = Fixture::new();
    let (mut broker, request, _) = setup(&fixture);
    let FileOperationRequest::Rename {
        idempotency_key, ..
    } = &request
    else {
        unreachable!()
    };
    let key = sha256(idempotency_key.as_bytes());
    let path = fixture.data.join(format!("idempotency/{key}.json"));
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(value["effect_at_ms"].is_i64());
    value["effect_at_ms"] = (now() + 5 * DAY).into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(pack(&mut broker, later()), 2);
}

/// 打了包的改名又冒出字节不同的散件：封不成，也开不了库。
#[test]
fn a_rename_record_that_differs_from_its_packed_copy_is_refused() {
    for part in [Part::FileRequest, Part::FileIntentAck, Part::Replay] {
        let fixture = Fixture::new();
        let (mut broker, _, _) = setup(&fixture);
        pack(&mut broker, later());
        super::seal::put_back(&fixture, part);
        let (broker, sealed) = checkpoint(&fixture, broker, "seal");
        assert!(sealed.is_err(), "{part:?}");
        drop(broker);
        assert!(fixture.try_reopen().is_err(), "{part:?}");
    }
}

/// 分在两段里的改名，保留字节、改名表都是两段加起来的。
#[test]
fn renames_in_two_segments_add_up() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    enable(&mut broker);
    for (index, offset) in [(0, 50), (1, 60)] {
        let request = rename(
            &broker,
            &format!("note-{index}.md"),
            &format!("moved-{index}.md"),
            &format!("move-{index}"),
        );
        run(&mut broker, request, offset);
        pack(&mut broker, later());
    }
    let archive = broker.archive().unwrap();
    assert_eq!((fixture.segments().len(), archive.renames().len()), (2, 2));
    let mut expected = 0;
    for pack in archive.packs() {
        for part in [Part::FileRequest, Part::FileIntentAck, Part::FileOutcomeAck] {
            if let Some(bytes) = archive.read(&broker.data, part, &pack.id).unwrap() {
                expected += bytes.len() as u64;
            }
        }
    }
    assert!(expected > 0);
    assert_eq!(archive.file_bytes(), expected);
}
