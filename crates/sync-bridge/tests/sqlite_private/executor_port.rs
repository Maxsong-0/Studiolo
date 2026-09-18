//! 生产持久层端口 `sqlite::StateJournal` 的行为契约，并为失败/恢复两片提供共享夹具。
//! 全部跑真实临时 SQLite；内存审计只验证顺序与 ACK 语义，不代表生产身份验收。

use super::support::*;
use crate::executor::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

/// 固定回执标识，与夹具 `committed()` 保持一致以便直接比对持久回执。
pub(super) const TERMINAL: &str = "synthetic-adapter-terminal-1";

/// 由持久层夹具的执行前证据构造执行器端口要的重验证据。
pub(super) fn recheck(store: &BridgeState, original: &Input, action: &Action) -> ExecutionRecheck {
    let check = check(store, original, action);
    ExecutionRecheck {
        binding: check.binding,
        source: check.source,
        target: check.target,
        canonical_observation: check.canonical_observation,
        mirror_observation: check.mirror_observation,
        witness_epoch: check.process_epoch,
        expires_at: check.expires_at,
    }
}

/// 由持久层夹具的完整共同观察构造端口要的收尾重查。
pub(super) fn common_recheck(
    store: &BridgeState,
    original: &Input,
    value: Presence,
) -> CommonRecheck {
    let observation = common(store, original, value);
    CommonRecheck {
        canonical: observation.canonical,
        mirror: observation.mirror,
        approved: observation.approved,
        witness_epoch: observation.process_epoch,
        expires_at: observation.expires_at,
    }
}

/// 与冻结动作逐字段一致的提交终态。
pub(super) fn commit(action: &Action) -> JobOutcome {
    let (new_revision, content_hash, bytes) = match &action.change {
        Change::Delete { .. } => (None, None, None),
        Change::CreateOnly { source } | Change::Replace { source, .. } => (
            Some(source.revision),
            Some(source.sha256),
            Some(source.bytes),
        ),
    };
    JobOutcome::Committed {
        new_revision,
        content_hash,
        bytes,
        receipt_id: TERMINAL.into(),
    }
}

/// 把整批审计推到没有积压为止，返回累计 ACK 条数。
pub(super) fn drain(journal: &mut dyn PlanJournal) -> usize {
    let mut total = 0;
    loop {
        let progress = journal.deliver_audit(64).unwrap();
        total += progress.acknowledged;
        if !progress.remaining {
            return total;
        }
    }
}

// ---------- 行为契约 ----------

#[test]
fn a_reserved_plan_is_reported_as_intent_pending_until_the_real_audit_acknowledges_it() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    let before = journal.snapshot().unwrap();
    assert_eq!(before.pending_sequence, Some(1));
    assert_eq!(before.baseline_version, 0);
    assert_eq!(before.retired_through, 0);
    assert!(!before.fused);
    assert_eq!(before.jobs.len(), 1);
    assert_eq!(before.jobs[0].key, action.key);
    assert_eq!(before.jobs[0].state, JobState::IntentPending);
    let persistent_key = before.jobs[0].external_key.clone();

    assert!(drain(&mut journal) > 0);
    let after = journal.snapshot().unwrap();
    assert_eq!(after.jobs[0].state, JobState::Reserved);
    // ACK 只推进 Intent，绝不更改后端幂等键。
    assert_eq!(after.jobs[0].external_key, persistent_key);
    assert!(after.version > before.version);
}

#[test]
fn issuing_then_recording_a_commit_settles_the_plan_and_advances_the_common_baseline() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let evidence = recheck(&store, &original, &action);
    let closing = common_recheck(&store, &original, present(9));
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    drain(&mut journal);
    let issued = journal.issue(&action.key, &evidence).unwrap();
    assert_eq!(issued.key, action.key);
    assert_eq!(
        issued.external_key,
        journal.snapshot().unwrap().jobs[0].external_key
    );
    assert_eq!(journal.snapshot().unwrap().jobs[0].state, JobState::Issued);

    journal.record_outcome(&issued, &commit(&action)).unwrap();
    assert_eq!(
        journal.snapshot().unwrap().jobs[0].state,
        JobState::Committed
    );
    drain(&mut journal);
    let version = journal.snapshot().unwrap().version;
    journal.settle(version, 1, Some(closing)).unwrap();

    let settled = journal.snapshot().unwrap();
    assert_eq!(settled.pending_sequence, None);
    assert_eq!(settled.baseline_version, 1);
    assert!(settled.jobs.is_empty());
}

#[test]
fn a_no_effect_outcome_is_persisted_as_no_effect_and_keeps_the_old_common_baseline() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let evidence = recheck(&store, &original, &action);
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());

    drain(&mut journal);
    let issued = journal.issue(&action.key, &evidence).unwrap();
    journal
        .record_outcome(
            &issued,
            &JobOutcome::NoEffect {
                reason: NoEffectCause::CancelledBeforeInstall,
                terminal_id: "synthetic-adapter-cancel-1".into(),
            },
        )
        .unwrap();
    assert_eq!(
        journal.snapshot().unwrap().jobs[0].state,
        JobState::NoEffect
    );

    drain(&mut journal);
    let version = journal.snapshot().unwrap().version;
    // 没有完整共同新观察时以 None 闭合：结果落库，共同基线保持不动。
    journal.settle(version, 1, None).unwrap();
    let settled = journal.snapshot().unwrap();
    assert_eq!(settled.pending_sequence, None);
    assert_eq!(settled.baseline_version, 0);
}
