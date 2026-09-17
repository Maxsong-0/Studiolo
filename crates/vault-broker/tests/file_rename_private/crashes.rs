use super::*;
#[test]
fn every_issued_boundary_reopens_unknown_and_preserves_known_target_effect() {
    for point in [72, 73, 74, 86, 87, 75, 76, 77, 78, 88, 89] {
        let (fixture, mut broker, revision) = setup(b"original");
        let r = request("note.md", "target.md", revision, &format!("fault-{point}"));
        pending(&mut broker, r.clone(), point);
        drop(broker);
        let mut broker = fixture.reopen();
        let phase = broker.file_operation_status().unwrap().unwrap().phase;
        let target_unknown = matches!(point, 72 | 73 | 74 | 86 | 87);
        let source_unknown = matches!(point, 76 | 77 | 78 | 88 | 89);
        let expected = if target_unknown {
            FileOperationPhase::TargetInstallIssued
        } else if point == 75 {
            FileOperationPhase::TargetInstalled
        } else {
            FileOperationPhase::SourceDetachIssued
        };
        assert_eq!(phase, expected);
        assert!(broker.write_status().unwrap().read_blocked);
        assert!(
            broker
                .file_operation_replay(&r, &metadata())
                .unwrap()
                .unwrap()
                .receipt
                .is_none()
        );
        let snapshot = broker.maintenance_inspect(1).unwrap().targets.remove(0);
        let known = &snapshot.file_operation.as_ref().unwrap().effects;
        assert_eq!(known.len(), 2);
        assert_eq!(
            matches!(known[0], FileEffect::Confirmed { .. }),
            !target_unknown
        );
        assert_eq!(
            matches!(known[1], FileEffect::Unknown { .. }),
            source_unknown
        );
        let source = fs::read(fixture.root.join("note.md")).ok();
        let target = fs::read(fixture.root.join("target.md")).ok();
        reconcile(&mut broker, MaintenanceDecision::PreserveInstalling);
        let result = receipt(&broker, &r);
        assert_eq!(result.outcome, FileOperationOutcome::Indeterminate);
        assert_eq!(result.effects, *known);
        assert!(result.evidence_id.is_some());
        assert_eq!(source, fs::read(fixture.root.join("note.md")).ok());
        assert_eq!(target, fs::read(fixture.root.join("target.md")).ok());
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
        drop(broker);
        assert_eq!(receipt(&fixture.reopen(), &r), result);
    }
}
#[test]
fn confirmed_both_paths_reopen_only_repairs_terminal_and_catalogs() {
    for point in [79, 80, 81, 58] {
        let (fixture, mut broker, revision) = setup(b"known");
        let r = request("note.md", "target.md", revision, "repair");
        pending(&mut broker, r.clone(), point);
        drop(broker);
        let mut broker = fixture.reopen();
        flush(&mut broker);
        let result = receipt(&broker, &r);
        assert_eq!(result.outcome, FileOperationOutcome::Committed);
        assert_eq!(
            broker
                .list_history(&path(), 10, None)
                .unwrap()
                .entries
                .len(),
            1
        );
        assert_eq!(
            broker
                .list_history(&logical("target.md"), 10, None)
                .unwrap()
                .entries
                .len(),
            1
        );
        assert_eq!(broker.list_tombstones(10, None).unwrap().entries.len(), 1);
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
        assert!(!fixture.root.join("note.md").exists());
    }
}
#[test]
fn every_preparation_checkpoint_can_be_cancelled_without_dropping_stage_reference() {
    for point in [70, 71, 83] {
        let (fixture, mut broker, revision) = setup(b"staged");
        let r = request("note.md", "target.md", revision, "preparing");
        fault(&broker, point);
        assert!(
            broker
                .prepare_file_operation(r.clone(), metadata())
                .is_err()
        );
        drop(broker);
        let mut broker = fixture.reopen();
        if point == 83 {
            let active = crate::mutation::load(&broker)
                .unwrap()
                .file_operation
                .unwrap();
            assert!(active.stage.is_some());
            assert!(active.rename.unwrap().target_revision.is_some());
        }
        reconcile(&mut broker, MaintenanceDecision::CancelPrepared);
        assert_eq!(
            receipt(&broker, &r).outcome,
            FileOperationOutcome::Cancelled
        );
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"staged");
        assert!(!fixture.root.join("target.md").exists());
    }
}
#[test]
fn target_installed_never_automatically_detaches_source_after_restart() {
    let (fixture, mut broker, revision) = setup(b"kept");
    let r = request("note.md", "target.md", revision, "half");
    pending(&mut broker, r.clone(), 75);
    drop(broker);
    for _ in 0..3 {
        let broker = fixture.reopen();
        assert!(
            broker
                .file_operation_replay(&r, &metadata())
                .unwrap()
                .unwrap()
                .receipt
                .is_none()
        );
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"kept");
        assert_eq!(fs::read(fixture.root.join("target.md")).unwrap(), b"kept");
    }
}

#[test]
fn partial_move_maintenance_crashes_only_resume_original_fixed_decision() {
    for point in [33, 34, 35] {
        let (fixture, mut broker, revision) = setup(b"reviewed");
        let r = request("note.md", "target.md", revision, "maintenance-crash");
        pending(&mut broker, r.clone(), 75);
        let target = broker.maintenance_inspect(1).unwrap().targets.remove(0);
        let ticket = broker
            .prepare_maintenance(
                &target.review_id,
                MaintenanceDecision::PreserveInstalling,
                MaintenanceActor::current().unwrap(),
            )
            .unwrap();
        broker
            .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
            .unwrap();
        fault(&broker, point);
        assert!(broker.commit_maintenance(ticket).is_err());
        drop(broker);
        let mut broker = fixture.reopen();
        if point == 33 {
            let ticket = broker
                .prepare_maintenance(
                    &target.review_id,
                    MaintenanceDecision::PreserveInstalling,
                    MaintenanceActor::current().unwrap(),
                )
                .unwrap();
            broker.commit_maintenance(ticket).unwrap();
        }
        flush(&mut broker);
        for candidate in broker.pending_maintenance_audit(2).unwrap() {
            broker
                .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
                .unwrap();
        }
        assert_eq!(
            receipt(&broker, &r).outcome,
            FileOperationOutcome::Indeterminate
        );
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"reviewed");
        assert_eq!(
            fs::read(fixture.root.join("target.md")).unwrap(),
            b"reviewed"
        );
        assert!(!broker.write_status().unwrap().unresolved);
    }
}
