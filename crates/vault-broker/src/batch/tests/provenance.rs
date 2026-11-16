use super::*;
use crate::{BrokerError, MutationActor, MutationKind, MutationMetadata};
fn metadata(instance: &str) -> MutationMetadata {
    MutationMetadata::new(
        MutationActor::Agent {
            agent_type: "claude".into(),
            instance_id: instance.into(),
            task_id: Some("task-1".into()),
            attempt_id: Some("attempt-2".into()),
        },
        MutationKind::Replace,
    )
    .unwrap()
}
#[test]
fn recorded_batch_has_recoverable_history_and_cannot_change_actor_on_replay() {
    let f = Fixture::new();
    let mut b = f.open();
    let r = request("recorded");
    let p = b.preview_batch(&r).unwrap();
    let receipt = b
        .commit_batch_recorded(r.clone(), &p, metadata("instance-1"))
        .unwrap();
    assert_eq!(
        b.commit_batch_recorded(r.clone(), &p, metadata("instance-1"))
            .unwrap(),
        receipt
    );
    assert!(matches!(
        b.commit_batch_recorded(r.clone(), &p, metadata("instance-2")),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert!(matches!(
        b.commit_batch(r, &p),
        Err(BrokerError::IdempotencyConflict)
    ));
    for item in &receipt.mutations {
        let entries = b.list_history(&item.path, 100, None).unwrap().entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].author, Some(metadata("instance-1").actor));
        assert!(entries[0].authored_at_ms.is_some());
        assert_eq!(
            b.read_history(&item.path, item.revision, 1024)
                .unwrap()
                .revision,
            item.revision
        );
    }
    b.rollback_batch_recorded("recorded", metadata("instance-2"))
        .unwrap();
    assert!(matches!(
        b.rollback_batch_recorded("recorded", metadata("instance-1")),
        Err(BrokerError::IdempotencyConflict)
    ));
    let entries = b
        .list_history(&path("notes/A.md"), 100, None)
        .unwrap()
        .entries;
    let changes = b.list_changes(100, None).unwrap();
    assert_eq!(changes.entries.len(), 4);
    assert_eq!(changes.entries[0].kind, crate::ChangeKind::BatchRollback);
    assert_eq!(changes.entries[0].actor, Some(metadata("instance-2").actor));
    assert_eq!(changes.entries[2].kind, crate::ChangeKind::Batch);
    assert_eq!(changes.entries[2].actor, Some(metadata("instance-1").actor));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].author, None);
    assert_eq!(entries[0].archived_by, Some(metadata("instance-2").actor));
    assert_eq!(entries[1].author, Some(metadata("instance-1").actor));
    drop(b);
    let b = f.reopen();
    assert_eq!(
        b.list_history(&path("notes/A.md"), 100, None)
            .unwrap()
            .entries,
        entries
    );
}
#[test]
fn completed_batch_history_resumes_after_each_durable_boundary() {
    for point in [43, 44, 45] {
        let f = Fixture::new();
        let mut b = f.open();
        let r = request("resume");
        let p = b.preview_batch(&r).unwrap();
        b.writer.fault.store(point, Ordering::Release);
        assert!(
            b.commit_batch_recorded(r.clone(), &p, metadata("instance-1"))
                .is_err()
        );
        assert!(matches!(
            b.list_history(&path("notes/A.md"), 100, None),
            Err(BrokerError::MutationUnresolved)
        ));
        drop(b);
        let mut b = f.reopen();
        b.commit_batch_recorded(r, &p, metadata("instance-1"))
            .unwrap();
        for name in ["notes/A.md", "notes/B.md"] {
            assert_eq!(
                b.list_history(&path(name), 100, None)
                    .unwrap()
                    .entries
                    .len(),
                1
            );
        }
    }
}
#[test]
fn rolling_back_before_publication_does_not_publish_the_cancelled_commit() {
    for point in [42, 44, 45] {
        let f = Fixture::new();
        let mut b = f.open();
        let r = request("cancel");
        let p = b.preview_batch(&r).unwrap();
        b.writer.fault.store(point, Ordering::Release);
        assert!(
            b.commit_batch_recorded(r, &p, metadata("instance-1"))
                .is_err()
        );
        b.rollback_batch_recorded("cancel", metadata("instance-2"))
            .unwrap();
        let entries = b
            .list_history(&path("notes/A.md"), 100, None)
            .unwrap()
            .entries;
        let changes = b.list_changes(100, None).unwrap();
        assert_eq!(changes.entries.len(), 2);
        assert!(
            changes
                .entries
                .iter()
                .all(|row| row.kind == crate::ChangeKind::BatchRollback)
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].author, None);
        assert_eq!(entries[0].archived_by, Some(metadata("instance-2").actor));
        assert!(matches!(
            b.read(&path("notes/A.md")),
            Err(BrokerError::NotFound(_))
        ));
    }
}
#[test]
fn history_cannot_forge_batch_actor_or_task_or_revision() {
    let f = Fixture::new();
    let mut b = f.open();
    let r = request("tamper");
    let p = b.preview_batch(&r).unwrap();
    b.commit_batch_recorded(r, &p, metadata("instance-1"))
        .unwrap();
    let catalog = f
        .data
        .join("history/meta-v1/catalogs")
        .join(format!("{}.json", crate::sha256(b"notes/A.md")));
    let original = fs::read(&catalog).unwrap();
    for field in ["instance_id", "task_id", "attempt_id"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["entries"][0]["entry"]["author"][field] = "forged".into();
        fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            b.list_history(&path("notes/A.md"), 100, None),
            Err(BrokerError::CorruptWriterState)
        ));
    }
    fs::write(&catalog, original).unwrap();
    assert_eq!(
        b.list_history(&path("notes/A.md"), 100, None)
            .unwrap()
            .entries
            .len(),
        1
    );
}
#[test]
fn agent_actor_rejects_unbound_attempt_and_control_data() {
    let mut actor = metadata("instance-1").actor;
    if let MutationActor::Agent { task_id, .. } = &mut actor {
        *task_id = None;
    }
    assert!(MutationMetadata::new(actor, MutationKind::Replace).is_err());
    assert!(
        MutationMetadata::new(
            MutationActor::Agent {
                agent_type: "claude".into(),
                instance_id: "bad/path".into(),
                task_id: None,
                attempt_id: None
            },
            MutationKind::Replace
        )
        .is_err()
    );
}

#[test]
fn memory_and_disk_recorded_batch_feed_share_the_visible_contract() {
    macro_rules! check {
        ($broker:expr) => {{
            let mut b = $broker;
            let r = request("paired");
            let preview = b.preview_batch(&r).unwrap();
            b.commit_batch_recorded(r.clone(), &preview, metadata("first"))
                .unwrap();
            let first = b.list_changes(1, None).unwrap();
            assert_eq!(first.entries.len(), 1);
            assert_eq!(first.through_sequence, 1);
            let second = b.list_changes(1, first.next_cursor.as_deref()).unwrap();
            assert_eq!(second.entries.len(), 1);
            assert_eq!(second.next_cursor, None);
            assert_eq!(first.entries[0].actor, Some(metadata("first").actor));
            assert_eq!(
                b.list_history(&path("notes/A.md"), 100, None)
                    .unwrap()
                    .entries
                    .len(),
                1
            );
            b.rollback_batch_recorded("paired", metadata("second"))
                .unwrap();
            assert!(matches!(
                b.commit_batch_recorded(r, &preview, metadata("first")),
                Err(BrokerError::MutationCancelled)
            ));
            let page = b.list_changes(100, None).unwrap();
            assert_eq!(page.entries.len(), 4);
            assert_eq!(page.through_sequence, 2);
            assert_eq!(page.entries[0].kind, crate::ChangeKind::BatchRollback);
            assert_eq!(page.entries[0].revision, None);
            assert_eq!(page.entries[2].kind, crate::ChangeKind::Batch);
            assert_eq!(page.entries[2].actor, Some(metadata("first").actor));
        }};
    }
    check!(MemoryVaultBroker::new());
    let fixture = Fixture::new();
    check!(fixture.open());
}
