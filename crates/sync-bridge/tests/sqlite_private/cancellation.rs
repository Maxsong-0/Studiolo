//! 冻结条件变化只终结从未发出的预约；真实 SQLite 重开与审计仍保留原键。
use super::support::*;
use crate::{sqlite::*, *};
use std::time::{Duration, Instant};
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn changed_unissued_source_or_target_cancels_durably_and_next_plan_uses_fresh_source() {
    for outbound in [false, true] {
        for target_changed in [false, true] {
            let fixture = Fixture::new();
            let mut store = fixture.init();
            let ledger = MemoryAuditLedger::new();
            let (canonical, mirror) = if outbound {
                (present(9), Presence::Missing)
            } else {
                (Presence::Missing, present(9))
            };
            let original = input(&store, 1, canonical, mirror);
            let row = reserve(&mut store, original.clone()).remove(0);
            let action = row.action;
            flush(&mut store, &ledger);
            let proof = check(&store, &original, &action);
            let version = store.view().unwrap().version;
            assert!(!store.cancel_changed(&action.key, proof.clone()).unwrap());
            assert_eq!(store.view().unwrap().version, version);
            let mut changed = proof;
            if target_changed {
                changed.target = present(8);
            } else {
                changed.source = present(8);
            }
            assert_eq!(
                store.issue(&action.key, changed.clone()),
                Err(BridgeError::StaleObservation)
            );
            assert!(store.cancel_changed(&action.key, changed).unwrap());
            let cancelled = store.lookup(&action.key).unwrap();
            assert_eq!(cancelled.external_key, row.external_key);
            let ActionStatus::Known(receipt) = &cancelled.status else {
                panic!("missing durable cancellation")
            };
            assert!(
                matches!(&receipt.outcome, ReceiptOutcome::NoEffect { reason: NoEffectReason::CancelledBeforeInstall, terminal_id, .. } if terminal_id.starts_with("bridge-unissued-cancel-"))
            );
            assert_eq!(
                receipt.backend,
                if outbound {
                    ReceiptBackend::Mirror {
                        mirror_root: [5; 32],
                    }
                } else {
                    ReceiptBackend::Broker {
                        canonical_root: [4; 32],
                    }
                }
            );
            drop(store);
            let mut store = fixture.reopen();
            assert_eq!(store.lookup(&action.key).unwrap(), cancelled);
            assert_eq!(
                store.issue(&action.key, check(&store, &original, &action)),
                Err(BridgeError::NotSettled)
            );
            assert_eq!(
                store.settle(store.view().unwrap().version, 1, None),
                Err(BridgeError::NotSettled)
            );
            flush(&mut store, &ledger);
            let common = common(&store, &original, present(8));
            assert_eq!(
                store.settle(store.view().unwrap().version, 1, Some(common)),
                Err(BridgeError::NotSettled)
            );
            store
                .settle(store.view().unwrap().version, 1, None)
                .unwrap();
            assert_eq!(store.view().unwrap().baseline.version, 0);
            let fresh = input(&store, 1, Presence::Missing, present(8));
            let next = reserve(&mut store, fresh).remove(0);
            assert_ne!(next.external_key, cancelled.external_key);
            assert_eq!(next.action.key.plan.sequence, 2);
            assert!(
                matches!(next.action.change, Change::CreateOnly { source } if source.sha256 == [8; 32])
            );
        }
    }
}

#[test]
fn cancellation_requires_intent_ack_fresh_bound_proof_and_never_changes_unknown() {
    let fixture = Fixture::new();
    let mut store = fixture.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let mut changed = check(&store, &original, &action);
    changed.source = present(8);
    assert_eq!(
        store.cancel_changed(&action.key, changed.clone()),
        Err(BridgeError::IntentRequired)
    );
    flush(&mut store, &ledger);
    let version = store.view().unwrap().version;
    for variant in 0..6 {
        let mut bad = changed.clone();
        let expected = match variant {
            0 => {
                bad.binding.scope_epoch += 1;
                BridgeError::BindingMismatch
            }
            1 => {
                bad.process_epoch = [0; 32];
                BridgeError::StaleObservation
            }
            2 => {
                bad.expires_at = Instant::now();
                BridgeError::StaleObservation
            }
            3 => {
                bad.canonical_observation = original.canonical.sequence;
                BridgeError::StaleObservation
            }
            4 => {
                bad.mirror_observation = original.mirror.sequence;
                BridgeError::StaleObservation
            }
            _ => {
                bad.mirror_observation = u64::MAX;
                BridgeError::StaleObservation
            }
        };
        assert_eq!(store.cancel_changed(&action.key, bad), Err(expected));
        assert_eq!(store.view().unwrap().version, version);
        assert_eq!(
            store.lookup(&action.key).unwrap().status,
            ActionStatus::Reserved
        );
    }
    store
        .issue(&action.key, check(&store, &original, &action))
        .unwrap();
    let row = store.lookup(&action.key).unwrap();
    assert_eq!(
        store.cancel_changed(&action.key, changed.clone()),
        Err(BridgeError::NotSettled)
    );
    assert_eq!(store.lookup(&action.key).unwrap(), row);
    drop(store);
    let mut store = fixture.reopen();
    assert_eq!(
        store.cancel_changed(&action.key, changed),
        Err(BridgeError::StaleObservation)
    );
    let mut fresh = check(&store, &original, &action);
    fresh.target = present(9);
    assert_eq!(
        store.cancel_changed(&action.key, fresh),
        Err(BridgeError::NotSettled)
    );
    assert_eq!(
        store.lookup(&action.key).unwrap().status,
        ActionStatus::OutcomeUnknown
    );
}

#[test]
fn cancellation_deadline_is_checked_under_lock_and_committed_terminal_survives_lost_reply() {
    for after_commit in [false, true] {
        let fixture = Fixture::new();
        let mut store = fixture.init();
        let ledger = MemoryAuditLedger::new();
        let original = input(&store, 1, Presence::Missing, present(9));
        let action = reserve(&mut store, original.clone()).remove(0).action;
        flush(&mut store, &ledger);
        let mut proof = check(&store, &original, &action);
        proof.source = present(8);
        proof.expires_at = Instant::now() + Duration::from_secs(1);
        let deadline = proof.expires_at;
        let wait = Box::new(move || {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            )
        });
        if after_commit {
            store.after_commit = Some(wait);
        } else {
            store.before_guard = Some(wait);
        }
        assert_eq!(
            store.cancel_changed(&action.key, proof),
            Err(BridgeError::StaleObservation)
        );
        assert!(store.after_commit.is_none() && store.before_guard.is_none());
        drop(store);
        let store = fixture.reopen();
        let status = store.lookup(&action.key).unwrap().status;
        if after_commit {
            assert!(matches!(status, ActionStatus::Known(_)));
        } else {
            assert_eq!(status, ActionStatus::Reserved);
        }
    }
}
