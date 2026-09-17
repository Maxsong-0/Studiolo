use super::*;

#[test]
fn reopened_residuals_are_removed_without_touching_published_or_private_records() {
    let f = Fixture::new();
    let b = f.open();
    let left = f.temporary("history", b"unfinished history");
    let right = f.temporary("snapshots", b"unfinished snapshot");
    let published = f.data.join("history").join("a".repeat(64));
    let evidence = f.data.join("snapshots/recovery-proof-previous.bin");
    let manifest = f.data.join("snapshots/recovery-proof.json");
    fs::write(&published, b"published").unwrap();
    fs::write(&evidence, b"evidence").unwrap();
    fs::write(&manifest, b"manifest").unwrap();
    fs::create_dir(f.data.join("snapshots/unrelated-directory")).unwrap();
    fs::write(f.data.join("snapshots/unrelated-directory/keep"), b"nested").unwrap();
    let state = fs::read(f.data.join("writer-state.json")).unwrap();
    drop(b);
    let mut b = f.reopen();
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 2,
            bytes: 37
        }
    );
    assert!(!left.exists() && !right.exists());
    assert_eq!(fs::read(published).unwrap(), b"published");
    assert_eq!(fs::read(evidence).unwrap(), b"evidence");
    assert_eq!(fs::read(manifest).unwrap(), b"manifest");
    assert_eq!(
        fs::read(f.data.join("snapshots/unrelated-directory/keep")).unwrap(),
        b"nested"
    );
    assert_eq!(fs::read(f.data.join("writer-state.json")).unwrap(), state);
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 0,
            bytes: 0
        }
    );
}

#[test]
fn interruption_reports_error_and_a_fresh_pass_finishes_only_remaining_temporaries() {
    for point in [40, 41] {
        let f = Fixture::new();
        let mut b = f.open();
        f.temporary("history", b"left");
        f.temporary("history", b"right");
        fault(&b, point);
        assert!(b.prune_blob_temporaries().is_err());
        assert_eq!(count(&f, "history"), if point == 40 { 2 } else { 1 });
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Ready);
        drop(b);
        let mut b = f.reopen();
        assert_eq!(
            b.prune_blob_temporaries().unwrap().removed,
            if point == 40 { 2 } else { 1 }
        );
        assert_eq!(count(&f, "history"), 0);
    }
}

#[test]
fn prepared_and_installing_refuse_the_entire_batch_and_retain_owned_stage() {
    for point in [1, 2, 3] {
        let f = Fixture::new();
        let mut b = f.open();
        fault(&b, point);
        assert!(
            b.mutate(Mutation::create(path(), b"new".to_vec(), "pending"))
                .is_err()
        );
        let stage = fs::read_dir(f.root.join(".workbench/staging"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        let left = f.temporary("history", b"left");
        let right = f.temporary("snapshots", b"right");
        drop(b);
        let mut b = f.reopen();
        let before = b.write_status().unwrap();
        assert_eq!(
            before.phase,
            if point == 1 {
                WriterPhase::Prepared
            } else {
                WriterPhase::Installing
            }
        );
        assert!(matches!(
            b.prune_blob_temporaries(),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(left.exists() && right.exists());
        assert!(stage.iter().all(|file| file.exists()));
        assert_eq!(b.write_status().unwrap(), before);
    }
}

#[test]
fn active_maintenance_still_blocks_when_the_original_operation_is_already_cancelled() {
    let f = Fixture::new();
    let mut b = f.open();
    fault(&b, 1);
    assert!(
        b.mutate(Mutation::create(path(), b"new".to_vec(), "pending"))
            .is_err()
    );
    let target = b.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = b
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    b.confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    b.commit_maintenance(ticket).unwrap();
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Ready);
    let residual = f.temporary("history", b"unfinished");
    assert!(b.prune_blob_temporaries().is_err());
    assert!(residual.exists());
    for candidate in b.pending_maintenance_audit(64).unwrap() {
        b.confirm_maintenance_audit(candidate, 2, &"a".repeat(64))
            .unwrap();
    }
    assert_eq!(b.prune_blob_temporaries().unwrap().removed, 1);
}

#[test]
fn ready_restore_outcome_requires_ack_but_ordinary_outbox_does_not_block_cleanup() {
    let f = Fixture::new();
    let mut b = f.open();
    let old = b
        .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
        .unwrap();
    let current = b
        .mutate_recorded(
            Mutation::replace(path(), b"new".to_vec(), old.revision, "new"),
            MutationMetadata::new(actor(), MutationKind::Replace).unwrap(),
        )
        .unwrap();
    f.temporary("history", b"ordinary");
    assert!(b.content_audit_status().unwrap().pending > 0);
    assert_eq!(b.prune_blob_temporaries().unwrap().removed, 1);
    let ticket = b
        .prepare_stream_restore(
            path(),
            current.revision,
            old.revision,
            "restore".into(),
            actor(),
            StreamLimits::new(crate::blob::OBJECT_LIMIT).unwrap(),
        )
        .unwrap();
    b.confirm_stream_restore_intent(&ticket, 3, &"a".repeat(64))
        .unwrap();
    b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Ready);
    assert!(b.content_audit_status().unwrap().restore_unconfirmed);
    let residual = f.temporary("snapshots", b"restore");
    assert!(matches!(
        b.prune_blob_temporaries(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(residual.exists());
}
