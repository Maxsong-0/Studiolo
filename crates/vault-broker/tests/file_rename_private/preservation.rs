use super::*;
#[test]
fn all_six_roles_preserve_actual_bytes_without_changing_historical_target_effect() {
    let (fixture, mut broker, revision) = setup(b"original");
    let r = request("note.md", "target.md", revision, "six");
    pending(&mut broker, r.clone(), 78);
    let state = crate::mutation::load(&broker).unwrap();
    let a = state.file_operation.unwrap();
    fs::write(fixture.root.join("note.md"), b"new source").unwrap();
    fs::write(fixture.root.join("target.md"), b"new target").unwrap();
    let stage = a.stage.unwrap();
    fs::write(
        fixture.root.join(".workbench/staging").join(stage.name),
        b"foreign stage",
    )
    .unwrap();
    let target = broker.maintenance_inspect(1).unwrap().targets.remove(0);
    let known = target.file_operation.as_ref().unwrap().effects.clone();
    assert!(matches!(
        &known[0],
        FileEffect::Confirmed {
            after: FilePresence::Present { bytes: 8, .. },
            ..
        }
    ));
    assert!(matches!(known[1], FileEffect::Unknown { .. }));
    reconcile(&mut broker, MaintenanceDecision::PreserveInstalling);
    let result = receipt(&broker, &r);
    assert_eq!(result.effects, known);
    let id = result.evidence_id.as_ref().unwrap();
    for (role, expected) in [
        ("source_current", b"new source".as_slice()),
        ("target_current", b"new target".as_slice()),
        ("source_original", b"original".as_slice()),
        ("target_proposed", b"original".as_slice()),
        ("source_quarantine", b"original".as_slice()),
        ("target_stage", b"foreign stage".as_slice()),
    ] {
        assert_eq!(
            fs::read(
                fixture
                    .data
                    .join("snapshots")
                    .join(format!("file-operation-{id}-{role}.bin"))
            )
            .unwrap(),
            expected
        );
    }
    let projected = Revision::compute(broker.vault_uuid, &logical("target.md"), b"original");
    assert!(
        ops::tombstone::protected(&broker, &logical("target.md"))
            .unwrap()
            .contains(&projected)
    );
    assert!(
        ops::tombstone::protected(&broker, &path())
            .unwrap()
            .contains(&revision)
    );
    assert_eq!(
        fs::read(fixture.root.join("note.md")).unwrap(),
        b"new source"
    );
    assert_eq!(
        fs::read(fixture.root.join("target.md")).unwrap(),
        b"new target"
    );
    assert_eq!(broker.list_tombstones(10, None).unwrap().entries.len(), 0);
}
#[test]
fn unobservable_either_current_path_or_stage_blocks_preservation_and_keeps_original_key() {
    for kind in [0, 1, 2] {
        let (fixture, mut broker, revision) = setup(b"old");
        let r = request("note.md", "target.md", revision, "unsafe");
        pending(&mut broker, r.clone(), 75);
        let active = crate::mutation::load(&broker)
            .unwrap()
            .file_operation
            .unwrap();
        let path = if kind == 0 {
            fixture.root.join("note.md")
        } else if kind == 1 {
            fixture.root.join("target.md")
        } else {
            fixture
                .root
                .join(".workbench/staging")
                .join(active.stage.unwrap().name)
        };
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        std::os::unix::fs::symlink("/not-a-real-rename-fixture-target", &path).unwrap();
        let review = broker.maintenance_inspect(1).unwrap().targets.remove(0);
        if kind < 2 {
            let observed = if kind == 0 {
                review.current
            } else {
                review.secondary.as_ref().unwrap().current
            };
            assert!(matches!(observed, MaintenanceCurrent::Unobservable { .. }));
        }
        assert!(matches!(
            broker.prepare_maintenance(
                &review.review_id,
                MaintenanceDecision::PreserveInstalling,
                MaintenanceActor::current().unwrap()
            ),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(
            broker
                .file_operation_replay(&r, &metadata())
                .unwrap()
                .unwrap()
                .receipt
                .is_none()
        );
        assert!(broker.write_status().unwrap().unresolved);
    }
}
#[test]
fn source_replacement_after_target_effect_is_rejected_without_any_detach() {
    let (fixture, mut broker, revision) = setup(b"same bytes");
    let r = request("note.md", "target.md", revision, "source-aba");
    pending(&mut broker, r, 75);
    let mut active = crate::mutation::load(&broker)
        .unwrap()
        .file_operation
        .unwrap();
    fs::write(fixture.root.join("fresh.md"), b"same bytes").unwrap();
    fs::rename(fixture.root.join("fresh.md"), fixture.root.join("note.md")).unwrap();
    assert!(ops::rename_source::prepare(&broker, &mut active).is_err());
    assert_eq!(
        broker.file_operation_status().unwrap().unwrap().phase,
        FileOperationPhase::TargetInstalled
    );
    assert_eq!(
        fs::read(fixture.root.join("note.md")).unwrap(),
        b"same bytes"
    );
}
#[test]
fn issued_gate_blocks_all_new_writes_and_prune_until_original_and_maintenance_ack() {
    let (_fixture, mut broker, revision) = setup(b"old");
    let r = request("note.md", "target.md", revision, "gate");
    pending(&mut broker, r, 75);
    assert!(matches!(
        broker.mutate(Mutation::create(
            logical("other.md"),
            b"new".to_vec(),
            "new"
        )),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.prune_stages(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.prune_blob_temporaries(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.prune_history(&path(), HistoryRetention::default(), 1),
        Err(BrokerError::MutationUnresolved)
    ));
    reconcile(&mut broker, MaintenanceDecision::PreserveInstalling);
    broker
        .mutate(Mutation::create(
            logical("other.md"),
            b"new".to_vec(),
            "new",
        ))
        .unwrap();
}
