use super::*;
use crate::{
    FileOperationLimits, FileOperationMetadata, FileOperationRequest, FileOperationScopeHash,
    FileOperationTicket, NewRootRebind,
};

fn restore(frozen: &FrozenBroker, fixture: &Fixture) -> FsVaultBroker {
    let left = fixture.target("restored-vault");
    let right = fixture.target("restored-data");
    let sealed = frozen.export(&left, &right).unwrap();
    let OwnerWatermark::Broker {
        vault_uuid,
        pair_id,
        ..
    } = sealed.watermark
    else {
        panic!("wrong owner");
    };
    let receipt = crate::rebind_new_roots(&NewRootRebind {
        new_vault_root: &left,
        new_data_root: &right,
        expected_vault_uuid: vault_uuid,
        expected_pair_id: pair_id,
        checkpoint_id: "checkpoint-optional-layouts".into(),
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    assert!(receipt.coverage.covers_required_bindings());
    FsVaultBroker::open_existing(&left, &right).unwrap()
}
#[test]
fn checkpoint_empty_vault_restores_without_inventing_history_metadata() {
    let fixture = Fixture::new();
    let mut frozen = fixture.open().freeze_checkpoint(deadline());
    assert_eq!(frozen.verify_audit(verify).unwrap().checked(), 0);
    let copy = restore(&frozen, &fixture);
    assert_eq!(copy.write_status().unwrap().committed_through, 0);
    assert!(crate::history::store::open(&copy, false).unwrap().is_none());
}
fn metadata() -> FileOperationMetadata {
    FileOperationMetadata::new(
        MutationActor::Obsidian,
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap()
}
fn complete(
    broker: &mut FsVaultBroker,
    ticket: &FileOperationTicket,
    offset: u64,
) -> crate::FileOperationReceipt {
    broker
        .confirm_file_operation_intent(ticket, offset, &"a".repeat(64))
        .unwrap();
    let receipt = broker.commit_file_operation(ticket).unwrap();
    for event in broker.pending_file_operation_audit(64).unwrap() {
        broker
            .confirm_file_operation_audit(event, offset + 1, &"a".repeat(64))
            .unwrap();
    }
    receipt
}
#[test]
fn checkpoint_deleted_body_restores_from_rebound_file_operation_storage() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    let limits = FileOperationLimits {
        effective_object_limit: crate::blob::OBJECT_LIMIT,
        max_tombstones: 64,
        retained_bytes_limit: 8 * 1024 * 1024 * 1024,
        minimum_free_bytes: 1024 * 1024,
    };
    broker.enable_file_operations(limits).unwrap();
    let ticket = broker
        .prepare_file_operation(
            FileOperationRequest::Delete {
                path: path(),
                expected: broker.read(&path()).unwrap().revision,
                idempotency_key: "deleted-before-backup".into(),
            },
            metadata(),
        )
        .unwrap();
    let deleted = complete(&mut broker, &ticket, 3);
    let mut frozen = broker.freeze_checkpoint(deadline());
    assert_eq!(frozen.verify_audit(verify).unwrap().checked(), 4);
    let mut copy = restore(&frozen, &fixture);
    assert_eq!(copy.file_operation_limits().unwrap(), Some(limits));
    let ticket = copy
        .prepare_file_operation(
            FileOperationRequest::RestoreTombstone {
                path: path(),
                tombstone_id: deleted.tombstone_id.unwrap(),
                idempotency_key: "restore-after-backup".into(),
            },
            metadata(),
        )
        .unwrap();
    complete(&mut copy, &ticket, 5);
    assert_eq!(copy.read(&path()).unwrap().content, "正文".as_bytes());
    // 原 owner 仍在且没有被恢复写入影响。
    assert!(matches!(
        frozen.broker.read(&path()),
        Err(BrokerError::NotFound(_))
    ));
}
