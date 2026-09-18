//! 恢复契约：重开、原键重放、未知终态、退休与持久熔断。

use super::ports::*;
use super::runner::*;
use super::support::*;
use crate::executor::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

fn prepared(store: &mut BridgeState, input: &Input) -> Plan {
    let frozen = ready(input);
    store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    frozen
}

fn copy_input(store: &BridgeState, count: usize) -> Input {
    let mut input = input(store, count);
    input.canonical.entries.insert(path(0), present(2));
    input
}

#[test]
fn reopening_replays_the_original_job_key_and_an_unknown_outcome_stays_unknown() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let input = copy_input(&store, 1);
    let plan = prepared(&mut store, &input);

    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    rig.backend.mode = BackendMode::Unknown;
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.issued, 1);
    assert_eq!(report.unknown, 1);
    assert_eq!(report.committed, 0);
    // 未知既不闭合计划，也不推进共同基线。
    assert!(!report.settled);
    assert!(!report.baseline_advanced);
    assert_eq!(store.view().unwrap().baseline.version, 1);
    let original_key = store.retry_pending(&input).unwrap()[0].external_key.clone();

    drop(store);
    let mut store = fixture.reopen();
    let reopened = store.retry_pending(&input).unwrap();
    assert_eq!(reopened[0].external_key, original_key);
    assert_eq!(reopened[0].status, ActionStatus::OutcomeUnknown);

    // 重开后再执行一次：沿用原键，不重发、不把相同内容补成成功。
    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.issued, 0);
    assert_eq!(report.already_known, 1);
    assert_eq!(report.unknown, 1);
    assert!(!report.settled);
    assert_eq!(rig.first("backend.apply"), None);
    assert_eq!(
        store.retry_pending(&input).unwrap()[0].external_key,
        original_key
    );
}

#[test]
fn an_unknown_copy_keeps_its_staged_object_instead_of_reclaiming_the_budget() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let input = copy_input(&store, 1);
    let plan = prepared(&mut store, &input);
    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    rig.backend.mode = BackendMode::Unknown;
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.unknown, 1);
    // 终态未知时既不声称已安装，也不回收暂存额度。
    let usage = rig.cas.usage();
    assert_eq!(usage.entries, 1);
    assert_eq!(usage.orphans, 1);
    assert_eq!(rig.cas.prepared(), None);
}

#[test]
fn a_no_effect_outcome_terminates_the_plan_but_keeps_the_old_common_baseline() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let input = copy_input(&store, 1);
    let plan = prepared(&mut store, &input);
    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    rig.backend.mode = BackendMode::Reject;
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.no_effect, 1);
    assert!(report.settled);
    assert!(!report.baseline_advanced);
    assert_eq!(store.view().unwrap().baseline.version, 1);
    assert_eq!(store.view().unwrap().pending_sequence, None);
    // 未安装的复制对象不留在 stage 里。
    assert_eq!(rig.cas.usage().entries, 0);
}

#[test]
fn an_aborted_transfer_leaves_no_half_object_and_no_issued_action() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let input = copy_input(&store, 1);
    let plan = prepared(&mut store, &input);
    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    rig.content.mode = ContentMode::Corrupt;
    assert_eq!(
        with_store(&mut store, &ledger, &input, &plan, &mut rig),
        Err(ExecutorError::Cas(CasFault::AddressMismatch))
    );
    assert_eq!(rig.cas.usage(), StageUsage::default());
    assert!(rig.sink.finished.is_empty());
    // Intent 已确认，但绝不会有 Issued。
    assert_eq!(
        store.retry_pending(&input).unwrap()[0].status,
        ActionStatus::Reserved
    );
}

#[test]
fn a_retired_sequence_is_permanently_refused_after_it_has_been_settled_and_retired() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);

    let first = copy_input(&store, 1);
    let first_plan = prepared(&mut store, &first);
    let mut rig = Rig::new(&first, store.observation_epoch(), Some(present(2)));
    assert!(
        with_store(&mut store, &ledger, &first, &first_plan, &mut rig)
            .unwrap()
            .settled
    );

    // 下一批闭合时，执行器先为上一份完整终态取得清理票据与真实审计 ACK。
    let mut second = input(&store, 1);
    second.canonical.entries.insert(path(0), present(3));
    let second_plan = prepared(&mut store, &second);
    let mut rig = Rig::new(&second, store.observation_epoch(), Some(present(3)));
    with_store(&mut store, &ledger, &second, &second_plan, &mut rig).unwrap();
    assert!(rig.first("journal.prepare_retirement").is_some());
    assert_eq!(
        store.view().unwrap().retired_through,
        first_plan.key().sequence
    );

    // 旧序号从此只被拒绝；拒绝不代表它曾经成功。
    let mut rig = Rig::new(&first, store.observation_epoch(), None);
    assert_eq!(
        with_store(&mut store, &ledger, &first, &first_plan, &mut rig),
        Err(ExecutorError::Retired)
    );
}

#[test]
fn a_planner_level_deletion_fuse_becomes_a_persistent_block_on_every_execution() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 50);
    let mut input = input(&store, 50);
    for index in 0..6 {
        input
            .canonical
            .entries
            .insert(path(index), Presence::Missing);
    }
    assert!(matches!(
        plan(input.clone(), None).unwrap(),
        Decision::FuseBlocked(_)
    ));
    let reservation = store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    assert!(matches!(reservation, Reservation::Fused { .. }));
    assert!(store.view().unwrap().fused);

    // 熔断是整批的：重开之后任何执行请求都直接被拒。
    drop(store);
    let mut store = fixture.reopen();
    let forged = forged_deletions(50, 1, true);
    let mut rig = Rig::new(forged.frozen_input(), store.observation_epoch(), None);
    assert_eq!(
        with_store(&mut store, &ledger, &input, &forged, &mut rig),
        Err(ExecutorError::Fused)
    );
}
