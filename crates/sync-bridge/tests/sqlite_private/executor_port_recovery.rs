//! 生产持久层端口 `sqlite::StateJournal` 的恢复契约：
//! 重开后原键重放、跨实例观察作废、退休票据与永久拒绝。

use super::executor_port::{commit, common_recheck, drain, recheck};
use super::support::*;
use crate::executor::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn reopening_the_store_replays_the_original_key_and_keeps_an_issued_action_unresolved() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    let stale = recheck(&store, &original, &action);
    let persistent_key = {
        let mut journal = StateJournal::new(&mut store, &ledger, original.clone());
        drain(&mut journal);
        journal.issue(&action.key, &stale).unwrap().external_key
    };

    drop(store);
    let mut store = fixture.reopen();
    let mut journal = StateJournal::new(&mut store, &ledger, original.clone());
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.jobs[0].state, JobState::Issued);
    assert_eq!(snapshot.jobs[0].external_key, persistent_key);
    // 上一实例的观察不能跨重开复用，也不能借重开重新 Issued。
    assert_eq!(
        journal.issue(&action.key, &stale),
        Err(JournalFault::StaleObservation)
    );
    assert_eq!(journal.snapshot().unwrap().jobs[0].state, JobState::Issued);
}

#[test]
fn closing_the_next_plan_needs_a_retirement_ticket_with_its_own_real_audit_acknowledgement() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();

    let first = input(&store, 1, Presence::Missing, present(9));
    let first_action = reserve(&mut store, first.clone()).remove(0).action;
    let first_evidence = recheck(&store, &first, &first_action);
    let first_closing = common_recheck(&store, &first, present(9));
    {
        let mut journal = StateJournal::new(&mut store, &ledger, first.clone());
        drain(&mut journal);
        let issued = journal.issue(&first_action.key, &first_evidence).unwrap();
        journal
            .record_outcome(&issued, &commit(&first_action))
            .unwrap();
        drain(&mut journal);
        let version = journal.snapshot().unwrap().version;
        journal.settle(version, 1, Some(first_closing)).unwrap();
    }

    let second = input(&store, 1, present(9), present(10));
    let second_action = reserve(&mut store, second.clone()).remove(0).action;
    let second_evidence = recheck(&store, &second, &second_action);
    let second_closing = common_recheck(&store, &second, present(10));
    let mut journal = StateJournal::new(&mut store, &ledger, second.clone());
    drain(&mut journal);
    let issued = journal.issue(&second_action.key, &second_evidence).unwrap();
    journal
        .record_outcome(&issued, &commit(&second_action))
        .unwrap();
    drain(&mut journal);

    let version = journal.snapshot().unwrap().version;
    assert_eq!(
        journal.settle(version, 2, Some(second_closing.clone())),
        Err(JournalFault::RetentionRequired)
    );
    journal.prepare_retirement(version).unwrap();
    // 票据本身也要一条真实审计 ACK；只取票不推审计仍然闭合不了。
    let ticketed = journal.snapshot().unwrap().version;
    assert_eq!(
        journal.settle(ticketed, 2, Some(second_closing.clone())),
        Err(JournalFault::RetentionRequired)
    );
    drain(&mut journal);
    let acknowledged = journal.snapshot().unwrap().version;
    journal
        .settle(acknowledged, 2, Some(second_closing))
        .unwrap();

    let settled = journal.snapshot().unwrap();
    assert_eq!(settled.retired_through, 1);
    assert_eq!(settled.baseline_version, 2);
}

#[test]
fn a_retired_plan_sequence_is_permanently_refused_after_reopening_the_store() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();

    let first = input(&store, 1, Presence::Missing, present(9));
    let first_action = reserve(&mut store, first.clone()).remove(0).action;
    let first_evidence = recheck(&store, &first, &first_action);
    let first_closing = common_recheck(&store, &first, present(9));
    {
        let mut journal = StateJournal::new(&mut store, &ledger, first.clone());
        drain(&mut journal);
        let issued = journal.issue(&first_action.key, &first_evidence).unwrap();
        journal
            .record_outcome(&issued, &commit(&first_action))
            .unwrap();
        drain(&mut journal);
        let version = journal.snapshot().unwrap().version;
        journal.settle(version, 1, Some(first_closing)).unwrap();
    }

    let second = input(&store, 1, present(9), present(10));
    let second_action = reserve(&mut store, second.clone()).remove(0).action;
    let second_evidence = recheck(&store, &second, &second_action);
    let second_closing = common_recheck(&store, &second, present(10));
    {
        let mut journal = StateJournal::new(&mut store, &ledger, second.clone());
        drain(&mut journal);
        let issued = journal.issue(&second_action.key, &second_evidence).unwrap();
        journal
            .record_outcome(&issued, &commit(&second_action))
            .unwrap();
        drain(&mut journal);
        let version = journal.snapshot().unwrap().version;
        journal.prepare_retirement(version).unwrap();
        drain(&mut journal);
        let version = journal.snapshot().unwrap().version;
        journal.settle(version, 2, Some(second_closing)).unwrap();
    }

    drop(store);
    let mut store = fixture.reopen();
    let journal = StateJournal::new(&mut store, &ledger, first);
    // 拒绝不代表它曾经成功：整批面仍可读，逐动作面永久为空。
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.retired_through, 1);
    assert!(snapshot.jobs.is_empty());
    assert_eq!(snapshot.baseline_version, 2);
    assert_eq!(journal.frozen_input().plan_sequence, 1);
}
