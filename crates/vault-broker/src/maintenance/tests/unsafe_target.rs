//! 原目标不安全时仍能取消未执行维护自身，不能拿旧审阅协调它。
use super::*;
use std::os::unix::fs::symlink;

fn replace_target(fixture: &Fixture, fifo: bool) {
    let target = fixture.root.join("note.md");
    fs::remove_file(&target).unwrap();
    if fifo {
        // rustix 的 mkfifoat 不在 macOS 暴露；沿用既有测试的系统 FIFO 夹具。
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&target)
                .status()
                .unwrap()
                .success()
        );
    } else {
        let outside = fixture._temp.path().join("outside.md");
        fs::write(&outside, b"outside must remain unchanged").unwrap();
        symlink(outside, target).unwrap();
    }
}

#[test]
fn unsafe_target_does_not_hide_fixed_slot_or_prevent_audit_and_own_cancellation() {
    for fifo in [false, true] {
        let fixture = Fixture::new();
        let (mut broker, request) = fixture.pending(1);
        let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
        let original = ticket.intent().clone();
        replace_target(&fixture, fifo);
        drop(broker);
        let mut broker = fixture.reopen();
        assert!(broker.maintenance_inspect(64).is_err());
        let pending = broker.maintenance_pending().unwrap().unwrap();
        assert_eq!(pending.intent, original);
        assert!(!pending.execution_started);
        assert!(!pending.intent_confirmed);
        flush(&mut broker);
        let pending = broker.maintenance_pending().unwrap().unwrap();
        assert!(pending.intent_confirmed);
        assert!(pending.outcome.is_none());
        let ticket = broker
            .prepare_maintenance(&original.target.review_id, original.decision, actor())
            .unwrap();
        assert!(broker.commit_maintenance(ticket).is_err());
        assert!(
            !broker
                .maintenance_pending()
                .unwrap()
                .unwrap()
                .execution_started
        );
        assert!(broker.mutation_replay(&request).unwrap().is_none());
        let receipt = broker
            .cancel_maintenance(&pending.maintenance_id, actor())
            .unwrap();
        assert_eq!(receipt.event.outcome, Some(MaintenanceOutcome::Cancelled));
        assert_eq!(receipt.event.resolution, None);
        assert!(
            broker
                .maintenance_pending()
                .unwrap()
                .unwrap()
                .outcome
                .is_some()
        );
        flush(&mut broker);
        assert!(broker.maintenance_pending().unwrap().is_none());
        assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Prepared);
        assert!(broker.mutation_replay(&request).unwrap().is_none());
        if !fifo {
            assert!(
                fs::symlink_metadata(fixture.root.join("note.md"))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                fs::read(fixture._temp.path().join("outside.md")).unwrap(),
                b"outside must remain unchanged"
            );
        }
    }
}

#[test]
fn unsafe_target_does_not_make_executing_maintenance_cancellable() {
    for fifo in [false, true] {
        let fixture = Fixture::new();
        let (mut broker, request) = fixture.pending(1);
        let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
        intent(&mut broker, &ticket);
        fault(&broker, 33);
        assert!(broker.commit_maintenance(ticket).is_err());
        replace_target(&fixture, fifo);
        drop(broker);
        let mut broker = fixture.reopen();
        let pending = broker.maintenance_pending().unwrap().unwrap();
        assert!(pending.execution_started);
        assert!(pending.outcome.is_none());
        flush(&mut broker);
        assert!(matches!(
            broker.cancel_maintenance(&pending.maintenance_id, actor()),
            Err(BrokerError::MutationUnresolved)
        ));
        let retry = broker
            .prepare_maintenance(
                &pending.intent.target.review_id,
                pending.intent.decision,
                actor(),
            )
            .unwrap();
        assert!(broker.commit_maintenance(retry).is_err());
        assert!(
            broker
                .maintenance_pending()
                .unwrap()
                .unwrap()
                .outcome
                .is_none()
        );
        assert!(broker.mutation_replay(&request).unwrap().is_none());
    }
}

#[test]
fn pending_only_observation_still_rejects_missing_slot_and_rebound_root() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let _ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    fs::remove_file(fixture.data.join("maintenance-v1/slot.json")).unwrap();
    assert!(broker.maintenance_pending().is_err());
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let _ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    fs::rename(&fixture.root, fixture.root.with_extension("moved")).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    assert!(broker.maintenance_pending().is_err());
}
