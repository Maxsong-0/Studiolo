use super::*;
#[test]
fn checked_reservation_accounts_two_l0_stage_quarantine_and_six_roles() {
    let bytes = ops::rename_capacity::reservation(13, true).unwrap();
    assert_eq!(bytes, 39 + 7 * crate::blob::OBJECT_LIMIT + 16 * 64 * 1024);
    assert!(matches!(
        ops::rename_capacity::reservation(u64::MAX, true),
        Err(BrokerError::WriterLimitExceeded)
    ));
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    fs::write(fixture.root.join("note.md"), b"x").unwrap();
    let revision = Revision::compute(broker.vault_uuid, &path(), b"x");
    let mut approved = limits();
    approved.retained_bytes_limit = 1024 * 1024;
    broker.enable_file_operations(approved).unwrap();
    broker.enable_file_rename().unwrap();
    assert!(matches!(
        broker.prepare_file_operation(
            request("note.md", "target.md", revision, "small"),
            metadata()
        ),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!broker.write_status().unwrap().unresolved);
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"x");
}
#[test]
fn snapshot_item_reservation_rejects_whole_batch_before_install() {
    let (fixture, mut broker, revision) = setup(b"old");
    crate::history::store::open(&broker, true).unwrap();
    let root = fixture.data.join("snapshots");
    for n in 0..49_994 {
        fs::write(root.join(format!("quota-{n}")), b"").unwrap();
    }
    assert!(matches!(
        broker.prepare_file_operation(
            request("note.md", "target.md", revision, "snapshot-full"),
            metadata()
        ),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(!fixture.root.join("target.md").exists());
}
#[test]
fn staged_checkpoint_keeps_projected_revision_referenced_after_cancel() {
    let (fixture, mut broker, revision) = setup(b"staged");
    let r = request("note.md", "target.md", revision, "stage-ref");
    fault(&broker, 83);
    assert!(matches!(
        broker.prepare_file_operation(r.clone(), metadata()),
        Err(BrokerError::Io {
            operation: "injected writer interruption",
            ..
        })
    ));
    let a = crate::mutation::load(&broker)
        .unwrap()
        .file_operation
        .unwrap();
    let target = a.rename.as_ref().unwrap().target_revision.unwrap();
    assert!(
        ops::tombstone::protected(&broker, &logical("target.md"))
            .unwrap()
            .contains(&target)
    );
    drop(broker);
    let mut broker = fixture.reopen();
    reconcile(&mut broker, MaintenanceDecision::CancelPrepared);
    assert!(
        ops::tombstone::protected(&broker, &logical("target.md"))
            .unwrap()
            .contains(&target)
    );
    assert_eq!(
        receipt(&broker, &r).outcome,
        FileOperationOutcome::Cancelled
    );
}
#[test]
fn completed_move_releases_unused_reservation_and_keeps_both_actual_l0_objects() {
    let (_fixture, mut broker, revision) = setup(b"kept");
    committed(
        &mut broker,
        request("note.md", "target.md", revision, "complete"),
    );
    let (actual, count) = ops::usage::actual(&broker).unwrap();
    assert_eq!(count, 1);
    assert!(actual < 1024 * 1024);
    let target = Revision::compute(broker.vault_uuid, &logical("target.md"), b"kept");
    assert!(
        ops::tombstone::protected(&broker, &logical("target.md"))
            .unwrap()
            .contains(&target)
    );
    assert!(
        ops::tombstone::protected(&broker, &path())
            .unwrap()
            .contains(&revision)
    );
}

#[test]
fn accepted_near_reservation_limit_can_finish_full_maintenance_and_reopen() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    fs::write(fixture.root.join("note.md"), b"x").unwrap();
    let revision = Revision::compute(broker.vault_uuid, &path(), b"x");
    let mut approved = limits();
    approved.retained_bytes_limit = ops::rename_capacity::reservation(1, true).unwrap() + 16 * 1024;
    broker.enable_file_operations(approved).unwrap();
    broker.enable_file_rename().unwrap();
    let r = request("note.md", "target.md", revision, "near-limit");
    pending(&mut broker, r.clone(), 76);
    drop(broker);
    let mut broker = fixture.reopen();
    reconcile(&mut broker, MaintenanceDecision::PreserveInstalling);
    let result = receipt(&broker, &r);
    assert_eq!(result.outcome, FileOperationOutcome::Indeterminate);
    assert!(ops::usage::actual(&broker).unwrap().0 < approved.retained_bytes_limit);
    drop(broker);
    assert_eq!(receipt(&fixture.reopen(), &r), result);
}
