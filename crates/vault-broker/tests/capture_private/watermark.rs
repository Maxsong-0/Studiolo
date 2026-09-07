use super::*;

#[test]
fn committed_mutation_invalidates_old_stamp_but_replay_does_not_advance_it() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let old = capture(&mut broker).stamp;
    let first = broker.mutate(request()).unwrap();
    assert!(matches!(
        broker.revalidate_capture(&old),
        Err(BrokerError::CaptureChanged)
    ));
    let current = capture(&mut broker);
    assert_eq!(current.stamp.committed_through(), 1);
    assert_eq!(broker.mutate(request()).unwrap(), first);
    broker.revalidate_capture(&current.stamp).unwrap();
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
    assert!(matches!(
        broker.mutate(Mutation::create(path("note.md"), b"no".to_vec(), "reject")),
        Err(BrokerError::RevisionConflict { .. })
    ));
    broker.revalidate_capture(&current.stamp).unwrap();
}

#[test]
fn successful_save_is_remembered_before_any_later_state_read_and_rollback_is_sticky() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let old = capture(&mut broker).stamp;
    let previous = fixture.journal();
    broker.mutate(request()).unwrap();
    let current = fixture.journal();
    // mutate 返回后未调用任何 Broker 读接口；成功 save 自身必须记住水位。
    fixture.restore_journal(&previous);
    assert!(matches!(
        broker.revalidate_capture(&old),
        Err(BrokerError::CorruptWriterState)
    ));
    fixture.restore_journal(&current);
    let before = inventory(fixture.temp.path());
    assert!(matches!(
        broker.write_status(),
        Err(BrokerError::CorruptWriterState)
    ));
    assert!(matches!(
        broker.capture(ScanLimits::default()),
        Err(BrokerError::CorruptWriterState)
    ));
    assert!(matches!(
        broker.read_file(&path("note.md"), 64),
        Err(BrokerError::CorruptWriterState)
    ));
    assert!(matches!(
        broker.mutate(request()),
        Err(BrokerError::CorruptWriterState)
    ));
    assert_eq!(inventory(fixture.temp.path()), before);
    drop(broker);
    // 不声称跨重启防回滚；新实例仍须从有效存储重新完整捕获。
    let mut reopened = fixture.open();
    assert_eq!(capture(&mut reopened).stamp.committed_through(), 1);
    assert!(matches!(
        reopened.revalidate_capture(&old),
        Err(BrokerError::CaptureChanged)
    ));
}

#[test]
fn validated_load_and_revalidation_observe_advances_even_when_stamp_is_rejected() {
    for through_status in [false, true] {
        let fixture = Fixture::new();
        let mut first = fixture.initialize();
        let previous = fixture.journal();
        first.mutate(request()).unwrap();
        let current = fixture.journal();
        drop(first);
        fixture.restore_journal(&previous);
        let mut broker = fixture.open();
        let old = capture(&mut broker).stamp;
        fixture.restore_journal(&current);
        if through_status {
            assert_eq!(broker.write_status().unwrap().committed_through, 1);
        } else {
            assert!(matches!(
                broker.revalidate_capture(&old),
                Err(BrokerError::CaptureChanged)
            ));
        }
        fixture.restore_journal(&previous);
        assert!(matches!(
            broker.revalidate_capture(&old),
            Err(BrokerError::CorruptWriterState)
        ));
    }
}

#[test]
fn reopening_observes_existing_commits_before_first_capture() {
    let fixture = Fixture::new();
    let mut first = fixture.initialize();
    let previous = fixture.journal();
    first.mutate(request()).unwrap();
    drop(first);
    let mut reopened = fixture.open();
    fixture.restore_journal(&previous);
    assert!(matches!(
        reopened.capture(ScanLimits::default()),
        Err(BrokerError::CorruptWriterState)
    ));
}

#[test]
fn invalid_journal_cannot_raise_the_highest_validated_watermark() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let original = fixture.journal();
    let mut invalid: serde_json::Value = serde_json::from_slice(&original).unwrap();
    invalid["committed_through"] = 99.into();
    // next_sequence 仍为 1，不满足日志不变量；不能把未验证的 99 记为已见提交。
    fixture.restore_journal(&serde_json::to_vec(&invalid).unwrap());
    assert!(matches!(
        broker.write_status(),
        Err(BrokerError::CorruptWriterState)
    ));
    fixture.restore_journal(&original);
    assert_eq!(capture(&mut broker).stamp.committed_through(), 0);
}

#[test]
fn artifact_and_manifest_installations_share_capture_watermark() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let old = capture(&mut broker).stamp;
    let request = crate::ArtifactCommit {
        task_id: crate::TaskId::new(),
        attempt_id: crate::AttemptId::new(),
        fence: crate::Fence::new(1).unwrap(),
        audit_intent: "fictional-capture".into(),
        idempotency_key: "capture-artifacts".into(),
        artifacts: vec![crate::ArtifactCandidate {
            name: path("result.md"),
            content: b"result".to_vec(),
        }],
    };
    let first = broker.commit_attempt_artifacts(request.clone()).unwrap();
    let current = capture(&mut broker);
    assert_eq!(current.stamp.committed_through(), 2);
    assert!(matches!(
        broker.revalidate_capture(&old),
        Err(BrokerError::CaptureChanged)
    ));
    assert_eq!(current.notes.len(), 1);
    assert!(
        current
            .entries
            .iter()
            .any(|entry| entry.path == first.manifest_path)
    );
    assert_eq!(broker.commit_attempt_artifacts(request).unwrap(), first);
    broker.revalidate_capture(&current.stamp).unwrap();
}
