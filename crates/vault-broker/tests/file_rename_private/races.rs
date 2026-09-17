//! 两个路径的外部竞争必须保留原字节和原请求，不允许把历史效果改成新观察。
use super::*;

#[test]
fn target_appearing_after_prepare_is_preserved_and_original_ticket_can_cancel() {
    for nested in [false, true] {
        let (fixture, mut broker, revision) = setup(b"source");
        if nested {
            fs::create_dir(fixture.root.join("sub")).unwrap();
        }
        let target = if nested { "sub/target.md" } else { "target.md" };
        let request = request("note.md", target, revision, "target-race");
        let ticket = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        fs::write(fixture.root.join(target), b"external target").unwrap();
        let result = broker.commit_file_operation(&ticket);
        if nested {
            assert!(
                matches!(result, Err(BrokerError::CaptureChanged)),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(BrokerError::UnstableRead(ref path)) if path == &logical("note.md")),
                "{result:?}"
            );
        }
        assert_eq!(
            broker.file_operation_status().unwrap().unwrap().phase,
            FileOperationPhase::Prepared
        );
        let cancelled = broker.cancel_file_operation(ticket).unwrap();
        assert_eq!(cancelled.outcome, FileOperationOutcome::Cancelled);
        assert!(
            cancelled
                .effects
                .iter()
                .all(|e| matches!(e, FileEffect::NotApplied { .. }))
        );
        flush(&mut broker);
        assert_eq!(receipt(&broker, &request), cancelled);
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"source");
        assert_eq!(
            fs::read(fixture.root.join(target)).unwrap(),
            b"external target"
        );
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
    }
}

#[test]
fn confirmed_target_inode_replacement_does_not_authorize_source_detach() {
    let (fixture, mut broker, revision) = setup(b"same bytes");
    let request = request("note.md", "target.md", revision, "target-aba");
    pending(&mut broker, request.clone(), 75);
    let mut active = crate::mutation::load(&broker)
        .unwrap()
        .file_operation
        .unwrap();
    fs::write(fixture.root.join("replacement.md"), b"same bytes").unwrap();
    fs::rename(
        fixture.root.join("replacement.md"),
        fixture.root.join("target.md"),
    )
    .unwrap();
    assert!(matches!(
        ops::rename_source::prepare(&broker, &mut active),
        Err(BrokerError::CaptureChanged)
    ));
    let current = broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert_eq!(current.phase, FileOperationPhase::TargetInstalled);
    assert!(current.receipt.is_none());
    assert_eq!(
        fs::read(fixture.root.join("note.md")).unwrap(),
        b"same bytes"
    );
    assert_eq!(
        fs::read(fixture.root.join("target.md")).unwrap(),
        b"same bytes"
    );
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
}

#[test]
fn replacement_target_parent_with_same_content_does_not_relax_identity_check() {
    let (fixture, mut broker, revision) = setup(b"source");
    fs::create_dir(fixture.root.join("sub")).unwrap();
    let request = request("note.md", "sub/target.md", revision, "parent-race");
    pending(&mut broker, request.clone(), 75);
    let mut active = crate::mutation::load(&broker)
        .unwrap()
        .file_operation
        .unwrap();
    fs::rename(fixture.root.join("sub"), fixture.root.join("retained-sub")).unwrap();
    fs::create_dir(fixture.root.join("sub")).unwrap();
    fs::write(fixture.root.join("sub/target.md"), b"source").unwrap();
    assert!(ops::rename_source::prepare(&broker, &mut active).is_err());
    assert_eq!(
        broker.file_operation_status().unwrap().unwrap().phase,
        FileOperationPhase::TargetInstalled
    );
    for path in ["note.md", "sub/target.md", "retained-sub/target.md"] {
        assert_eq!(fs::read(fixture.root.join(path)).unwrap(), b"source");
    }
    assert!(
        broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .is_none()
    );
}

#[test]
fn deeper_source_detach_preserves_target_ancestor_identity_without_self_cancellation() {
    let (fixture, mut broker, _) = setup(b"nested");
    fs::create_dir_all(fixture.root.join("sub/deep")).unwrap();
    fs::rename(
        fixture.root.join("note.md"),
        fixture.root.join("sub/deep/a.md"),
    )
    .unwrap();
    let from = logical("sub/deep/a.md");
    let to = logical("sub/b.md");
    let revision = Revision::compute(broker.vault_uuid, &from, b"nested");
    let moved = committed(
        &mut broker,
        request(from.as_str(), to.as_str(), revision, "nested"),
    );
    assert_eq!(moved.outcome, FileOperationOutcome::Committed);
    assert!(!fixture.root.join(from.as_str()).exists());
    assert_eq!(fs::read(fixture.root.join(to.as_str())).unwrap(), b"nested");
    assert_eq!(
        broker.list_history(&from, 10, None).unwrap().entries.len(),
        1
    );
    assert_eq!(broker.list_history(&to, 10, None).unwrap().entries.len(), 1);
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
}
