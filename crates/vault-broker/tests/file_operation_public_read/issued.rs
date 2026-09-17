//! 实际摘名或安装后仍须原持久事实与审计闭合，重开不借正文推成成功。
use super::*;

#[test]
fn fault54_restored_bytes_are_private_until_explicit_preservation_and_both_ack_layers() {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(b"old");
    let historical = broker.snapshot(&inputs(revision)).unwrap();
    let deleted = committed(&mut broker, delete(revision, "delete"));
    let request = restore(deleted.tombstone_id.as_deref().unwrap(), "restore-unknown");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    fault(&broker, 54);
    assert!(matches!(
        broker.commit_file_operation(&ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    denied(&broker, &fixture, revision);
    // 固定历史快照和原删除终态不冒充当前观察，仍可读取。
    assert_eq!(broker.load_snapshot(&historical.id).unwrap(), historical);
    assert_eq!(
        broker
            .file_operation_replay(&delete(revision, "delete"), &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .unwrap(),
        deleted
    );
    drop(broker);

    let mut broker = fixture.reopen();
    denied(&broker, &fixture, revision);
    let status = broker.file_operation_status().unwrap().unwrap();
    assert_eq!(status.phase, FileOperationPhase::InstallIssued);
    assert!(status.receipt.is_none());
    assert!(broker.pending_file_operation_audit(64).unwrap().is_empty());
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::PreserveInstalling,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    denied(&broker, &fixture, revision);
    let original = broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert_eq!(
        original.receipt.unwrap().outcome,
        FileOperationOutcome::Indeterminate
    );
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
    flush(&mut broker);
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
            .unwrap();
    }
    assert!(!broker.write_status().unwrap().unresolved);
    readable(&broker, revision);
}

#[test]
fn issued_delete_cannot_publish_either_the_old_body_or_an_unconfirmed_missing_fact() {
    for point in [52, 54] {
        let fixture = Fixture::new();
        let (mut broker, revision) = fixture.seed(b"old");
        let ticket = broker
            .prepare_file_operation(delete(revision, "delete-unknown"), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        fault(&broker, point);
        assert!(matches!(
            broker.commit_file_operation(&ticket),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fixture.root.join("note.md").exists(), point == 52);
        denied(&broker, &fixture, revision);
        drop(broker);
        let broker = fixture.reopen();
        denied(&broker, &fixture, revision);
        assert_eq!(
            broker.file_operation_status().unwrap().unwrap().phase,
            FileOperationPhase::DetachIssued
        );
    }
}

#[test]
fn confirmed_restore_requires_outcome_ack_before_publication_live_and_after_reopen() {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(b"old");
    let deleted = committed(&mut broker, delete(revision, "delete"));
    let request = restore(deleted.tombstone_id.as_deref().unwrap(), "restore");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Committed);
    denied(&broker, &fixture, revision);
    assert_eq!(
        broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .unwrap(),
        receipt
    );
    drop(broker);
    let mut broker = fixture.reopen();
    denied(&broker, &fixture, revision);
    assert_eq!(
        broker.file_operation_status().unwrap().unwrap().phase,
        FileOperationPhase::FinalizedAwaitAudit
    );
    flush(&mut broker);
    assert_eq!(broker.write_status().unwrap().committed_through, 2);
    readable(&broker, revision);
}
