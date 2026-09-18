//! 生产持久层端口 `sqlite::StateJournal` 的失败契约：
//! 映射穷尽、门禁前置、未知不落回执、审计不可用、租约冲突与持久熔断。

use super::executor_port::{commit, drain, recheck};
use super::support::*;
use crate::executor::*;
use crate::sqlite::executor_port::journal_fault;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn every_bridge_error_variant_has_an_explicit_journal_fault_and_none_becomes_a_success() {
    // `journal_fault` 用穷尽 match 写成：新增 BridgeError 变体会直接编译失败，
    // 这张表只核对每个变体当前落在哪一类，不允许悄悄换类。
    let table = [
        (BridgeError::VersionConflict, JournalFault::VersionConflict),
        (BridgeError::NotFound, JournalFault::NotFound),
        (BridgeError::Retired, JournalFault::Retired),
        (BridgeError::Fused, JournalFault::Fused),
        (BridgeError::IntentRequired, JournalFault::IntentRequired),
        (BridgeError::OutcomeMismatch, JournalFault::OutcomeMismatch),
        (
            BridgeError::StaleObservation,
            JournalFault::StaleObservation,
        ),
        (BridgeError::Budget, JournalFault::Budget),
        (BridgeError::BindingMismatch, JournalFault::BindingMismatch),
        (BridgeError::NotSettled, JournalFault::NotSettled),
        (
            BridgeError::AuditUnavailable,
            JournalFault::AuditUnavailable,
        ),
        (
            BridgeError::RetentionRequired,
            JournalFault::RetentionRequired,
        ),
        (BridgeError::MissingStore, JournalFault::Unavailable),
        (BridgeError::AlreadyExists, JournalFault::Unavailable),
        (BridgeError::UnsafeStorage, JournalFault::Unavailable),
        (BridgeError::Busy, JournalFault::Unavailable),
        (BridgeError::UnsupportedPlatform, JournalFault::Unavailable),
        (BridgeError::UnsupportedSchema, JournalFault::Unavailable),
        (BridgeError::CorruptStore, JournalFault::Unavailable),
        (BridgeError::Unavailable, JournalFault::Unavailable),
        (BridgeError::Overflow, JournalFault::Unavailable),
        (BridgeError::PendingUnresolved, JournalFault::Unavailable),
        (BridgeError::RetryMismatch, JournalFault::Unavailable),
        (BridgeError::OutcomeUnknown, JournalFault::Unavailable),
    ];
    assert_eq!(table.len(), 24, "BridgeError 变体数变了就必须同步这张表");
    for (error, expected) in table {
        assert_eq!(journal_fault(error.clone()), expected, "{error:?}");
    }
}

#[test]
fn issuing_before_the_real_intent_acknowledgement_is_refused_as_intent_required() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let evidence = recheck(&store, &original, &action);
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    assert_eq!(
        journal.issue(&action.key, &evidence),
        Err(JournalFault::IntentRequired)
    );
    assert_eq!(
        journal.snapshot().unwrap().jobs[0].state,
        JobState::IntentPending
    );
}

#[test]
fn an_unknown_outcome_is_refused_instead_of_being_persisted_as_an_effect() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let evidence = recheck(&store, &original, &action);
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    drain(&mut journal);
    let issued = journal.issue(&action.key, &evidence).unwrap();
    assert_eq!(
        journal.record_outcome(&issued, &JobOutcome::Unknown),
        Err(JournalFault::OutcomeMismatch)
    );
    // 未知既不写回执也不倒回 Reserved：原键停在 Issued，等下次按原键核对。
    assert_eq!(journal.snapshot().unwrap().jobs[0].state, JobState::Issued);
    let version = journal.snapshot().unwrap().version;
    assert_eq!(
        journal.settle(version, 1, None),
        Err(JournalFault::NotSettled)
    );
}

#[test]
fn an_audit_adapter_that_never_acknowledges_surfaces_as_audit_unavailable() {
    let fixture = Fixture::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    reserve(&mut store, original.clone());
    let mut journal = StateJournal::new(&mut store, &RejectAudit, original.clone());

    assert_eq!(
        journal.deliver_audit(64),
        Err(JournalFault::AuditUnavailable)
    );
    assert_eq!(
        journal.snapshot().unwrap().jobs[0].state,
        JobState::IntentPending
    );
    // 越界的 flush 预算是调用方错误，不是审计不可用。
    assert_eq!(journal.deliver_audit(0), Err(JournalFault::Budget));
    assert_eq!(journal.deliver_audit(65), Err(JournalFault::Budget));
}

#[test]
fn recording_an_outcome_for_a_key_outside_the_frozen_plan_is_refused_as_not_found() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 2, Presence::Missing, present(9));
    let mut views = reserve(&mut store, original.clone());
    let action = views.remove(0).action;
    let evidence = recheck(&store, &original, &action);
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    drain(&mut journal);
    let issued = journal.issue(&action.key, &evidence).unwrap();
    let mut forged = issued.clone();
    forged.key = views.remove(0).action.key;
    // 另一条真实存在但未 Issued 的动作：持久层拒绝把别人的回执安到它头上。
    assert_eq!(
        journal.record_outcome(&forged, &commit(&action)),
        Err(JournalFault::OutcomeMismatch)
    );

    let mut absent = issued;
    absent.key.plan.sequence += 7;
    assert_eq!(
        journal.record_outcome(&absent, &commit(&action)),
        Err(JournalFault::NotFound)
    );
}

#[test]
fn a_second_owner_cannot_take_the_store_while_the_lease_is_held_and_that_stays_unknown() {
    let fixture = Fixture::new();
    let held = fixture.init();
    assert_eq!(
        BridgeState::open_existing(&fixture.root, binding()).map(|_| ()),
        Err(BridgeError::Busy)
    );
    drop(held);
    // 租约冲突在端口上只能是「拿不到权威结论」，绝不是一个可据以推进的已知类别。
    assert_eq!(journal_fault(BridgeError::Busy), JournalFault::Unavailable);
    assert!(BridgeState::open_existing(&fixture.root, binding()).is_ok());
}

#[test]
fn a_planner_level_deletion_fuse_is_visible_in_the_snapshot_and_blocks_every_settlement() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let seed = input(&store, 6, present(9), present(9));
    assert!(reserve(&mut store, seed.clone()).is_empty());
    flush(&mut store, &ledger);
    let closing = common(&store, &seed, present(9));
    store
        .settle(store.view().unwrap().version, 1, Some(closing))
        .unwrap();

    let deleting = input(&store, 6, Presence::Missing, present(9));
    assert!(matches!(
        store.reserve(store.view().unwrap().version, deleting.clone()),
        Ok(Reservation::Fused { .. })
    ));
    let mut journal = StateJournal::new(&mut store, &ledger, deleting);

    let snapshot = journal.snapshot().unwrap();
    assert!(snapshot.fused);
    // 熔断是整批的：没有逐动作面可走，任何闭合都被永久挡住。
    assert!(snapshot.jobs.is_empty());
    assert_eq!(
        journal.settle(snapshot.version, 2, None),
        Err(JournalFault::Fused)
    );
    assert_eq!(
        journal.prepare_retirement(snapshot.version),
        Err(JournalFault::Fused)
    );
}
