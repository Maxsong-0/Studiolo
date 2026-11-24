use super::*;
#[test]
fn checkpoint_checks_permanent_content_acks_even_after_queue_cleanup() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    assert_eq!(broker.content_audit_status().unwrap().pending, 0);
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let capture = frozen.take_capture().unwrap();
    let mut phases = Vec::new();
    capture
        .export(
            &fixture.target("copy-vault"),
            &fixture.target("copy-data"),
            deadline(),
        )
        .unwrap()
        .verify(deadline(), |event, offset, hash| {
            let BrokerCheckpointAudit::Content(event) = event else {
                panic!("wrong audit class")
            };
            assert_eq!(event.path, path());
            assert_eq!(
                offset,
                if event.phase == ContentAuditPhase::Intent {
                    1
                } else {
                    2
                }
            );
            verify(BrokerCheckpointAudit::Content(event), offset, hash)?;
            phases.push(event.phase);
            Ok(())
        })
        .unwrap();
    assert_eq!(phases.len(), 2);
    let directory = fixture.data.join("history/meta-v1/requests");
    let ack = fs::read_dir(&directory)
        .unwrap()
        .map(|v| v.unwrap().path())
        .find(|v| v.to_string_lossy().ends_with("-outcome.json.ack"))
        .unwrap();
    fs::remove_file(ack).unwrap();
    let left = fixture.target("left");
    let right = fixture.target("right");
    assert!(seal(&mut frozen, &left, &right).is_err());
    // 母本和两份 ACK 都被删去时，仍由永久终态的反向引用发现缺失。
    for entry in fs::read_dir(directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let left = fixture.target("gone-left");
    let right = fixture.target("gone-right");
    assert!(seal(&mut frozen, &left, &right).is_err());
}
#[test]
fn checkpoint_checks_file_operation_and_completed_maintenance_receipts() {
    let fixture = Fixture::new();
    let broker = full_history(&fixture);
    assert!(!broker.write_status().unwrap().unresolved);
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let capture = frozen.take_capture().unwrap();
    let left = fixture.target("left");
    let right = fixture.target("right");
    let mut classes = [0; 3];
    let (sealed, coverage) = capture
        .export(&left, &right, deadline())
        .unwrap()
        .verify(deadline(), |event, offset, hash| {
            classes[match &event {
                BrokerCheckpointAudit::Content(_) => 0,
                BrokerCheckpointAudit::FileOperation(_) => 1,
                BrokerCheckpointAudit::Maintenance(_) => 2,
            }] += 1;
            verify(event, offset, hash)
        })
        .unwrap();
    assert_eq!(classes, [2, 2, 2]);
    assert_eq!((coverage.checked(), coverage.max_offset()), (6, 6));
    assert!(sealed.is_verified());
    // 和直接在活的写者上核的结果一样。
    assert_eq!(verify_live(&frozen.broker, verify).unwrap(), coverage);
    let OwnerWatermark::Broker {
        vault_uuid,
        pair_id,
        ..
    } = frozen.watermark().unwrap()
    else {
        panic!("wrong owner");
    };
    crate::rebind_new_roots(&crate::NewRootRebind {
        new_vault_root: &left,
        new_data_root: &right,
        expected_vault_uuid: vault_uuid,
        expected_pair_id: pair_id,
        checkpoint_id: "checkpoint-maintenance".into(),
        restore_id: "restore-maintenance",
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    let copy = FsVaultBroker::open_existing(&left, &right).unwrap();
    assert!(!copy.write_status().unwrap().unresolved);
}
