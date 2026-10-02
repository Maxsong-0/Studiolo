//! 陈旧预约在正文保全/写入之前取消；其余动作继续，原基线保持。
use super::{ports::*, runner::*, support::*};
use crate::{executor::*, sqlite::*};
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn changed_sources_cancel_before_staging_on_both_sides_without_calling_backend() {
    for outbound in [false, true] {
        let fixture = Fixture::new();
        let ledger = MemoryAuditLedger::new();
        let mut store = fixture.init();
        seed(&mut store, &ledger, 2);
        let mut input = input(&store, 2);
        let source = if outbound {
            &mut input.canonical
        } else {
            &mut input.mirror
        };
        for i in 0..2 {
            source.entries.insert(path(i), present(2));
        }
        let plan = ready(&input);
        store
            .reserve(store.view().unwrap().version, input.clone())
            .unwrap();
        let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(3)));
        let source = if outbound {
            &mut rig.witness.input.canonical
        } else {
            &mut rig.witness.input.mirror
        };
        for i in 0..2 {
            source.entries.insert(path(i), present(3));
        }
        rig.content.mode = ContentMode::Unavailable;
        let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
        assert_eq!(
            (
                report.issued,
                report.committed,
                report.no_effect,
                report.unknown
            ),
            (0, 0, 2, 0)
        );
        assert!(report.settled && !report.baseline_advanced);
        assert!(rig.backend.staged.is_empty() && rig.sink.finished.is_empty());
        assert!(rig.first("open ").is_none() && rig.first("journal.issue").is_none());
        assert_eq!(store.view().unwrap().baseline.version, 1);
    }
}

#[test]
fn changed_unissued_job_does_not_block_other_jobs_and_reopen_does_not_replay_it() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 2);
    let mut input = input(&store, 2);
    for i in 0..2 {
        input.mirror.entries.insert(path(i), present(2));
    }
    let plan = ready(&input);
    store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    rig.witness.input.mirror.entries.insert(path(0), present(3));
    rig.backend.mode = BackendMode::Unknown;
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!((report.issued, report.no_effect, report.unknown), (1, 1, 1));
    assert!(!report.settled);
    assert_eq!(rig.backend.staged.len(), 1);
    let rows = store.retry_pending(&input).unwrap();
    assert!(matches!(rows[0].status, ActionStatus::Known(_)));
    assert_eq!(rows[1].status, ActionStatus::OutcomeUnknown);
    drop(store);
    let mut store = fixture.reopen();
    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(
        (
            report.issued,
            report.no_effect,
            report.unknown,
            report.already_known
        ),
        (0, 1, 1, 2)
    );
    assert!(rig.backend.staged.is_empty());
    assert_eq!(store.retry_pending(&input).unwrap(), rows);
}

#[test]
fn missing_observation_never_becomes_cancellation() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let mut input = input(&store, 1);
    input.mirror.entries.insert(path(0), present(2));
    let plan = ready(&input);
    store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    rig.witness.fault = Some(WitnessFault::Incomplete);
    assert_eq!(
        with_store(&mut store, &ledger, &input, &plan, &mut rig),
        Err(ExecutorError::Witness(WitnessFault::Incomplete))
    );
    assert_eq!(
        store.retry_pending(&input).unwrap()[0].status,
        ActionStatus::Reserved
    );
    assert!(rig.backend.staged.is_empty());
}
