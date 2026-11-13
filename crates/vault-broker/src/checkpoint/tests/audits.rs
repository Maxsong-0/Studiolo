use super::*;
use crate::{
    FileOperationLimits, FileOperationMetadata, FileOperationRequest, FileOperationScopeHash,
    MaintenanceActor, MaintenanceDecision,
};
#[test]
fn checkpoint_checks_permanent_content_acks_even_after_queue_cleanup() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    assert_eq!(broker.content_audit_status().unwrap().pending, 0);
    let mut frozen = broker.freeze_checkpoint(deadline());
    let mut phases = Vec::new();
    frozen
        .verify_audit(|event, offset, hash| {
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
    assert!(frozen.verify_audit(verify).is_err());
    let left = fixture.target("left");
    let right = fixture.target("right");
    assert!(frozen.export(&left, &right).is_err());
    // 母本和两份 ACK 都被删去时，仍由永久终态的反向引用发现缺失。
    for entry in fs::read_dir(directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    assert!(frozen.verify_audit(verify).is_err());
}
#[test]
fn checkpoint_checks_file_operation_and_completed_maintenance_receipts() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: crate::blob::OBJECT_LIMIT,
            max_tombstones: 64,
            retained_bytes_limit: 8 * 1024 * 1024 * 1024,
            minimum_free_bytes: 1024 * 1024,
        })
        .unwrap();
    let metadata = FileOperationMetadata::new(
        MutationActor::Obsidian,
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap();
    let original = broker
        .prepare_file_operation(
            FileOperationRequest::Delete {
                path: path(),
                expected: broker.read(&path()).unwrap().revision,
                idempotency_key: "abandoned".into(),
            },
            metadata,
        )
        .unwrap();
    broker
        .confirm_file_operation_intent(&original, 3, &"a".repeat(64))
        .unwrap();
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&ticket, 4, &"a".repeat(64))
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    for event in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(event, 5, &"a".repeat(64))
            .unwrap();
    }
    for event in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(event, 6, &"a".repeat(64))
            .unwrap();
    }
    assert!(!broker.write_status().unwrap().unresolved);
    let mut frozen = broker.freeze_checkpoint(deadline());
    let mut classes = [0; 3];
    let coverage = frozen
        .verify_audit(|event, offset, hash| {
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
    let left = fixture.target("left");
    let right = fixture.target("right");
    assert!(frozen.export(&left, &right).unwrap().is_verified());
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
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    let copy = FsVaultBroker::open_existing(&left, &right).unwrap();
    assert!(!copy.write_status().unwrap().unresolved);
}
