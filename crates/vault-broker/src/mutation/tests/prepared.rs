use super::*;

#[test]
fn prepared_reopens_and_only_the_exact_original_request_can_continue() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    interrupt(&broker, 1);
    assert!(broker.mutate(request()).is_err());
    assert!(!fixture.root.join("note.md").exists());
    assert_blocked(&mut broker);
    let mut changed = request();
    changed.content = b"different".to_vec();
    assert!(matches!(
        broker.mutate(changed),
        Err(BrokerError::IdempotencyConflict)
    ));
    drop(broker);
    let mut reopened = fixture.open();
    assert!(reopened.write_status().unwrap().unresolved);
    let receipt = reopened.mutate(request()).unwrap();
    assert_eq!(receipt.previous_revision, None);
    assert_eq!(reopened.write_status().unwrap().committed_through, 1);
    assert!(!reopened.write_status().unwrap().unresolved);
    assert_eq!(
        fs::read_dir(fixture.root.join(".workbench/staging"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn prepared_rechecks_actual_cas_and_remembers_rejection_after_source_is_restored() {
    for interrupted_cancel in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        let first = broker
            .mutate(Mutation::create(path("note.md"), b"old".to_vec(), "old"))
            .unwrap();
        let replacement =
            Mutation::replace(path("note.md"), b"new".to_vec(), first.revision, "replace");
        interrupt(&broker, 1);
        assert!(broker.mutate(replacement.clone()).is_err());
        assert_eq!(
            fs::read(
                fixture
                    .data
                    .join("history")
                    .join(first.revision.to_string())
            )
            .unwrap(),
            b"old"
        );
        fs::write(fixture.root.join("note.md"), b"external").unwrap();
        if interrupted_cancel {
            interrupt(&broker, 6);
        }
        assert!(broker.mutate(replacement.clone()).is_err());
        // 恢复原始字节也不能把已持久拒绝的键重新授权。
        fs::write(fixture.root.join("note.md"), b"old").unwrap();
        drop(broker);
        let mut reopened = fixture.open();
        assert!(!reopened.write_status().unwrap().unresolved);
        assert!(matches!(
            reopened.mutate(replacement),
            Err(BrokerError::RevisionConflict { .. })
        ));
        assert_eq!(reopened.read(&path("note.md")).unwrap().content, b"old");
        assert_eq!(reopened.write_status().unwrap().committed_through, 1);
        assert_eq!(
            fs::read_dir(fixture.root.join(".workbench/staging"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn malformed_operation_cannot_advance_the_watermark() {
    for damage in ["sequence", "receipt", "stage-name", "target"] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        interrupt(&broker, 1);
        assert!(broker.mutate(request()).is_err());
        let mut state = load(&broker).unwrap();
        let operation = state.operation.as_mut().unwrap();
        match damage {
            "sequence" => operation.sequence = 0,
            "receipt" => operation.receipt = Some(operation.receipt()),
            "stage-name" => operation.stage.name = "../escape".into(),
            _ => {
                operation.target_revision =
                    crate::Revision::compute(broker.vault_uuid(), &path("note.md"), b"different")
            }
        }
        fs::write(
            fixture.data.join("writer-state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(broker.mutate(request()).is_err());
        assert!(!fixture.root.join("note.md").exists());
    }
}
