use super::*;
#[test]
fn explicit_enable_limits_and_missing_or_partial_schema_fail_closed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let expected = Revision::compute(broker.vault_uuid, &path(), b"old");
    assert!(
        broker
            .prepare_file_operation(delete(expected, "disabled"), metadata())
            .is_err()
    );
    broker.enable_file_operations(limits()).unwrap();
    assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 5);
    let mut changed = limits();
    changed.max_tombstones -= 1;
    assert!(broker.enable_file_operations(changed).is_err());
    drop(broker);
    fs::remove_file(fixture.data.join("file-operations-v1/binding.json")).unwrap();
    assert!(FsVaultBroker::open_existing(&fixture.root, &fixture.data).is_err());
    let fixture = Fixture::new();
    drop(fixture.open());
    fs::create_dir(fixture.data.join("file-operations-v1.preparing")).unwrap();
    assert!(FsVaultBroker::open_existing(&fixture.root, &fixture.data).is_err());
}
#[test]
fn source_and_scope_binding_survive_replay_and_reopen() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let request = delete(original, "binding");
    let receipt = committed(&mut broker, request.clone());
    let mut other = metadata();
    other.scope_epoch = 2;
    assert!(matches!(
        broker.file_operation_replay(&request, &other),
        Err(BrokerError::IdempotencyConflict)
    ));
    let other_request = FileOperationRequest::RestoreTombstone {
        path: path(),
        tombstone_id: receipt.tombstone_id.unwrap(),
        idempotency_key: "binding".into(),
    };
    assert!(matches!(
        broker.file_operation_replay(&other_request, &metadata()),
        Err(BrokerError::IdempotencyConflict)
    ));
    drop(broker);
    let mut broker = fixture.reopen();
    let ticket = broker.prepare_file_operation(request, metadata()).unwrap();
    fs::write(fixture.root.join("note.md"), b"later").unwrap();
    assert_eq!(
        broker.commit_file_operation(&ticket).unwrap().outcome,
        FileOperationOutcome::Committed
    );
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"later");
}
#[test]
fn retained_capacity_and_object_limit_fail_before_any_active_or_install() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    fs::write(fixture.root.join("note.md"), b"old").unwrap();
    let revision = Revision::compute(broker.vault_uuid, &path(), b"old");
    let mut configured = limits();
    configured.retained_bytes_limit = 1024;
    broker.enable_file_operations(configured).unwrap();
    assert!(matches!(
        broker.prepare_file_operation(delete(revision, "too-small"), metadata()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(broker.file_operation_status().unwrap().is_none());
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
}
#[test]
fn old_state_target_and_event_json_remain_byte_compatible() {
    let pair: VaultUuid = "11111111-1111-4111-8111-111111111111".parse().unwrap();
    let state = crate::mutation::State::initial(pair, false);
    assert_eq!(
        serde_json::to_string(&state).unwrap(),
        "{\"format_version\":1,\"pair_id\":\"11111111-1111-4111-8111-111111111111\",\"committed_through\":0,\"next_sequence\":1,\"legacy_unresolved\":false,\"operation\":null}"
    );
    let revision = Revision::compute(pair, &path(), b"old");
    let target = MaintenanceTarget {
        review_id: "1".repeat(64),
        request_fingerprint: "2".repeat(64),
        path: path(),
        kind: MaintenanceTargetKind::Prepared,
        sequence: Some(1),
        previous_revision: Some(revision),
        target_revision: Some(revision),
        file_operation_id: None,
        secondary: None,
        file_operation: None,
        current: MaintenanceCurrent::Missing,
        proposed_available: true,
    };
    let encoded = serde_json::to_string(&target).unwrap();
    // 旧版 Revision 的透明 serde 表示是 32 字节数组；固定值不依赖被测 DTO。
    let old_revision = "[96,66,88,28,206,29,174,165,94,89,53,244,85,155,234,177,19,242,226,155,225,121,186,110,52,246,251,86,84,145,49,92]";
    let golden = format!(
        "{{\"review_id\":\"{}\",\"request_fingerprint\":\"{}\",\"path\":\"note.md\",\"kind\":\"prepared\",\"sequence\":1,\"previous_revision\":{old_revision},\"target_revision\":{old_revision},\"current\":{{\"kind\":\"missing\"}},\"proposed_available\":true}}",
        "1".repeat(64),
        "2".repeat(64)
    );
    assert_eq!(encoded, golden);
    let event = MaintenanceAuditEvent {
        schema_version: None,
        event_id: "3".repeat(64),
        phase: ContentAuditPhase::Intent,
        vault_uuid: pair,
        actor: MaintenanceActor::LocalOs { uid: 501, gid: 20 },
        decision: MaintenanceDecision::CancelPrepared,
        requested_at_ms: Some(5),
        target,
        outcome: None,
        resolution: None,
        evidence_id: None,
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        format!(
            "{{\"event_id\":\"{}\",\"phase\":\"intent\",\"vault_uuid\":\"{pair}\",\"actor\":{{\"kind\":\"local_os\",\"uid\":501,\"gid\":20}},\"decision\":\"cancel_prepared\",\"requested_at_ms\":5,\"target\":{golden},\"outcome\":null,\"resolution\":null,\"evidence_id\":null}}",
            "3".repeat(64)
        )
    );
}
#[test]
fn protected_paths_cross_instance_tickets_and_root_replacement_are_rejected() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let bad = FileOperationRequest::Delete {
        path: VaultPath::parse(".workbench/private").unwrap(),
        expected: original,
        idempotency_key: "bad".into(),
    };
    assert!(broker.prepare_file_operation(bad, metadata()).is_err());
    let ticket = broker
        .prepare_file_operation(delete(original, "ticket"), metadata())
        .unwrap();
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.confirm_file_operation_intent(&ticket, 1, &"a".repeat(64)),
        Err(BrokerError::CaptureChanged)
    ));
    fs::rename(&fixture.root, fixture.root.with_extension("moved")).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    assert!(broker.file_operation_status().is_err());
}
