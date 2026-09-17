//! 有未决证据时不进行预检协调，持久拒绝后的故障也不复活请求。

use super::*;

#[test]
fn check_keeps_all_pending_phases_and_stage_bytes_untouched() {
    for point in [1, 2, 3, 4] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(point);
        let journal = fs::read(f.data.join("writer-state.json")).unwrap();
        let current = fs::read(f.root.join("note.md")).unwrap();
        let stage = f.stage(&broker);
        let staged = fs::read(&stage).ok();
        assert!(matches!(
            broker.check_precondition(&request),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fs::read(f.data.join("writer-state.json")).unwrap(), journal);
        assert_eq!(fs::read(f.root.join("note.md")).unwrap(), current);
        assert_eq!(fs::read(stage).ok(), staged);
    }
}

#[test]
fn legacy_and_uncertain_cannot_create_precheck_terminal() {
    for legacy in [true, false] {
        let f = Fixture::new();
        let mut broker = f.initialize();
        if legacy {
            let mut state = crate::mutation::load(&broker).unwrap();
            state.legacy_unresolved = true;
            fs::write(
                f.data.join("writer-state.json"),
                serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
        } else {
            broker.writer.poison();
        }
        let request = Mutation::create(path(), b"unused".to_vec(), "request");
        assert!(matches!(
            broker.check_precondition(&request),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fs::read_dir(f.data.join("idempotency")).unwrap().count(), 0);
    }
}

#[test]
fn failure_after_rejection_persistence_keeps_original_missing_outcome_after_reopen() {
    let f = Fixture::new();
    let mut broker = f.initialize();
    let first = broker
        .mutate(Mutation::create(path(), b"A".to_vec(), "first"))
        .unwrap();
    fs::remove_file(f.root.join("note.md")).unwrap();
    let request = Mutation::replace(path(), b"B".to_vec(), first.revision, "missing");
    interrupt(&broker, 6);
    assert!(matches!(
        broker.check_precondition(&request),
        Err(BrokerError::Io { .. })
    ));
    fs::write(f.root.join("note.md"), b"A").unwrap();
    drop(broker);
    let mut reopened = f.open();
    assert!(matches!(
        reopened.check_precondition(&request),
        Err(BrokerError::RevisionConflict { actual: None, .. })
    ));
    assert_eq!(fs::read(f.root.join("note.md")).unwrap(), b"A");
    assert_eq!(reopened.write_status().unwrap().committed_through, 1);
}

#[test]
fn cancelled_and_indeterminate_checks_replay_original_terminal() {
    for point in [1, 2] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(point);
        let ticket = broker.pending_mutation().unwrap().unwrap();
        let decision = if point == 1 {
            PendingDecision::CancelPrepared
        } else {
            preserve()
        };
        broker.reconcile_pending(&ticket, decision).unwrap();
        let result = broker.check_precondition(&request);
        if point == 1 {
            assert!(matches!(result, Err(BrokerError::MutationCancelled)));
        } else {
            assert!(matches!(
                result,
                Err(BrokerError::MutationIndeterminate { .. })
            ));
        }
    }
}

#[test]
fn all_non_success_terminal_fields_stay_bound_to_original_request() {
    use crate::mutation::{
        record::{Outcome, Terminal},
        replay::{self, Replay},
    };
    for forged_precondition in [false, true] {
        let f = Fixture::new();
        let broker = f.initialize();
        let request = Mutation::create(path(), b"new".to_vec(), "request");
        let target = crate::Revision::compute(broker.vault_uuid(), &path(), b"new");
        let other = crate::Revision::compute(broker.vault_uuid(), &path(), b"old");
        let stored_path = if forged_precondition {
            path()
        } else {
            VaultPath::parse("wrong.md").unwrap()
        };
        let stored_condition = if forged_precondition {
            crate::Precondition::Match(other)
        } else {
            request.precondition
        };
        for outcome in [
            Outcome::Rejected {
                path: stored_path.clone(),
                precondition: stored_condition,
                actual: Some(target),
            },
            Outcome::Cancelled {
                path: stored_path.clone(),
                precondition: stored_condition,
            },
            Outcome::Indeterminate {
                path: stored_path,
                precondition: stored_condition,
                evidence_id: "a".repeat(64),
                current_revision: None,
            },
        ] {
            let format_version = if matches!(outcome, Outcome::Rejected { .. }) {
                2
            } else {
                3
            };
            let record = Terminal {
                recording: None,
                format_version,
                pair_id: broker.writer.binding.pair_id,
                sequence: None,
                fingerprint: FsVaultBroker::fingerprint(&request).unwrap(),
                expected_revision: target,
                outcome,
            };
            record.validate(broker.writer.binding.pair_id).unwrap();
            assert!(matches!(
                replay::matches(
                    Replay::Current(record),
                    &FsVaultBroker::fingerprint(&request).unwrap(),
                    target,
                    &path(),
                    request.precondition
                ),
                Err(BrokerError::CorruptWriterState)
            ));
        }
    }
}
