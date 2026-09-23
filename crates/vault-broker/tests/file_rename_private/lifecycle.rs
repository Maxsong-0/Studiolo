use super::*;
#[test]
fn explicit_enable_preserves_old_frozen_bytes_and_never_advances_sequence() {
    let fixture = Fixture::new();
    let (mut broker, revision) = fixture.seed(b"original");
    let rename = request("note.md", "target.md", revision, "rename");
    assert!(matches!(
        broker.prepare_file_operation(rename, metadata()),
        Err(BrokerError::InvalidArtifact(reason)) if reason == "重命名尚未显式启用"
    ));
    let old =
        ops::frozen::Frozen::new(&broker, &delete(revision, "old"), &metadata(), limits()).unwrap();
    let before = crate::mutation::load(&broker).unwrap();
    assert_eq!(before.format_version, 5);
    assert!(!before.rename_enabled);
    assert!(
        !serde_json::to_string(&before)
            .unwrap()
            .contains("rename_enabled")
    );
    broker.enable_file_rename().unwrap();
    broker.enable_file_operations(limits()).unwrap();
    let after = crate::mutation::load(&broker).unwrap();
    assert_eq!(after.format_version, 6);
    assert!(after.rename_enabled);
    assert_eq!(
        (after.next_sequence, after.committed_through),
        (before.next_sequence, before.committed_through)
    );
    let mut again =
        ops::frozen::Frozen::new(&broker, &delete(revision, "old"), &metadata(), limits()).unwrap();
    again.requested_at_ms = old.requested_at_ms;
    assert_eq!(
        serde_json::to_vec(&old).unwrap(),
        serde_json::to_vec(&again).unwrap()
    );
    let deleted = committed(&mut broker, delete(revision, "delete"));
    let terminal = ops::terminal::load(
        &broker,
        &ops::request::load(&broker, &deleted.operation_id)
            .unwrap()
            .unwrap()
            .frozen,
    )
    .unwrap()
    .unwrap();
    assert_eq!(terminal.format_version, 5);
    assert!(terminal.rename.is_none());
    drop(broker);
    assert!(
        crate::mutation::load(&fixture.reopen())
            .unwrap()
            .rename_enabled
    );
}
#[test]
fn same_directory_and_shared_ancestor_moves_have_two_revisions_and_one_watermark() {
    for to in ["target.md", "sub/target.md"] {
        let (fixture, mut broker, revision) = setup("# 原字节\n".as_bytes());
        if to.contains('/') {
            fs::create_dir(fixture.root.join("sub")).unwrap();
        }
        let r = request("note.md", to, revision, "move");
        let result = committed(&mut broker, r.clone());
        let target_revision =
            Revision::compute(broker.vault_uuid, &logical(to), "# 原字节\n".as_bytes());
        assert_ne!(revision, target_revision);
        assert_eq!(result.sequence, 1);
        assert_eq!(result.effects.len(), 2);
        assert!(!fixture.root.join("note.md").exists());
        assert_eq!(
            fs::read(fixture.root.join(to)).unwrap(),
            "# 原字节\n".as_bytes()
        );
        assert!(
            matches!(&result.effects[0],FileEffect::Confirmed{path,before:FilePresence::Missing,after:FilePresence::Present{revision:r,..}} if path==&logical(to)&&*r==target_revision)
        );
        assert!(
            matches!(&result.effects[1],FileEffect::Confirmed{path,after:FilePresence::Missing,..} if path==&logical("note.md"))
        );
        let source = broker.list_history(&path(), 10, None).unwrap();
        let target = broker.list_history(&logical(to), 10, None).unwrap();
        assert_eq!((source.entries.len(), target.entries.len()), (1, 1));
        assert_ne!(source.entries[0].id, target.entries[0].id);
        assert_eq!(
            broker
                .read_history(&logical(to), target_revision, 1024)
                .unwrap()
                .content,
            "# 原字节\n".as_bytes()
        );
        assert_eq!(receipt(&broker, &r), result);
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
        let tombstone = broker.list_tombstones(10, None).unwrap().entries.remove(0);
        assert_eq!(tombstone.path, path());
        assert_eq!(tombstone.revision, revision);
        if let (Some(start), Some(end)) = (tombstone.deleted_at_ms, tombstone.retain_until_ms) {
            assert!(end - start >= 7 * 86_400_000);
        }
        committed(&mut broker, restore(&tombstone.id, "restore-source"));
        assert_eq!(
            fs::read(fixture.root.join("note.md")).unwrap(),
            fs::read(fixture.root.join(to)).unwrap()
        );
    }
}
#[test]
fn reverse_move_is_a_new_request_but_old_receipt_never_reexecutes() {
    let (fixture, mut broker, revision) = setup(b"aba");
    let first = request("note.md", "target.md", revision, "first");
    let initial = committed(&mut broker, first.clone());
    let target = Revision::compute(broker.vault_uuid, &logical("target.md"), b"aba");
    let second = committed(
        &mut broker,
        request("target.md", "note.md", target, "second"),
    );
    assert_eq!(second.sequence, 2);
    let replay = broker
        .prepare_file_operation(first.clone(), metadata())
        .unwrap();
    assert_eq!(broker.commit_file_operation(&replay).unwrap(), initial);
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"aba");
    assert!(!fixture.root.join("target.md").exists());
    assert!(matches!(
        broker.prepare_file_operation(
            request("note.md", "another.md", revision, "first"),
            metadata()
        ),
        Err(BrokerError::IdempotencyConflict)
    ));
}
#[test]
fn missing_source_and_existing_empty_target_are_fixed_rejections() {
    for missing in [false, true] {
        let (fixture, mut broker, revision) = setup(b"old");
        if missing {
            fs::remove_file(fixture.root.join("note.md")).unwrap();
        } else {
            fs::write(fixture.root.join("target.md"), b"").unwrap();
        }
        let r = request("note.md", "target.md", revision, "reject");
        let ticket = broker
            .prepare_file_operation(r.clone(), metadata())
            .unwrap();
        assert_eq!(
            ticket.preview().fixed_outcome,
            Some(FileOperationOutcome::Rejected)
        );
        intent(&mut broker, &ticket);
        let result = broker.commit_file_operation(&ticket).unwrap();
        flush(&mut broker);
        assert!(
            result
                .effects
                .iter()
                .all(|e| matches!(e, FileEffect::NotApplied { .. }))
        );
        assert_eq!(result.outcome, FileOperationOutcome::Rejected);
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
    }
}

#[test]
fn enabling_interruption_preserves_either_whole_v5_or_whole_v6_marker() {
    for point in [84, 85] {
        let fixture = Fixture::new();
        let (mut broker, _) = fixture.seed(b"unchanged");
        fault(&broker, point);
        assert!(matches!(
            broker.enable_file_rename(),
            Err(BrokerError::Io {
                operation: "injected writer interruption",
                ..
            })
        ));
        drop(broker);
        let broker = fixture.reopen();
        let state = crate::mutation::load(&broker).unwrap();
        assert_eq!(state.rename_enabled, point == 85);
        assert_eq!(state.format_version, if point == 85 { 6 } else { 5 });
        assert_eq!((state.next_sequence, state.committed_through), (1, 0));
        assert_eq!(
            fs::read(fixture.root.join("note.md")).unwrap(),
            b"unchanged"
        );
    }
}
