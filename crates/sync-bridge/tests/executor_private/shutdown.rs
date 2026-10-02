//! 大批次关停：已发动作保存终态，未发动作和审计 outbox 沿原计划恢复。
use super::{fake_journal::FakeJournal, ports::*, runner::*, support::*};
use crate::{executor::*, sqlite::*, *};
use std::time::{Duration, Instant};
use studiolo_audit_ledger::MemoryAuditLedger;

#[derive(Clone, Copy)]
enum StopWhen {
    Immediate,
    AfterAudit,
    AfterApply,
}
struct StopBackend<'a> {
    inner: &'a mut FakeBackend,
    when: StopWhen,
}
impl JobBackend for StopBackend<'_> {
    fn stopping(&self) -> bool {
        match self.when {
            StopWhen::Immediate => true,
            StopWhen::AfterAudit => self
                .inner
                .log
                .borrow()
                .iter()
                .any(|line| line == "audit.flush"),
            StopWhen::AfterApply => !self.inner.staged.is_empty(),
        }
    }
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        self.inner.apply(job, action, staged)
    }
}
fn run(
    rig: &mut Rig,
    plan: &Plan,
    journal: &mut dyn PlanJournal,
    when: StopWhen,
) -> Result<ExecutionReport, ExecutorError> {
    let mut backend = StopBackend {
        inner: &mut rig.backend,
        when,
    };
    let now = Instant::now();
    execute(
        plan,
        ExecutionPorts {
            quiescence: &rig.quiescence,
            journal,
            witness: &mut rig.witness,
            content: &mut rig.content,
            sink: &mut rig.sink,
            backend: &mut backend,
            cas: &mut rig.cas,
        },
        now,
        now + Duration::from_secs(60),
    )
}
fn two_actions() -> Input {
    let mut input = raw_input(2, 1, true);
    input.mirror.entries.insert(path(0), present(2));
    input.mirror.entries.insert(path(1), present(2));
    input
}
#[test]
fn stop_before_execution_keeps_all_jobs_and_audit_untouched() {
    let input = two_actions();
    let plan = ready(&input);
    let mut rig = Rig::new(&input, [9; 32], None);
    let mut journal = FakeJournal::for_plan(&plan, rig.log.clone());
    journal.audit_pending = 200;
    assert_eq!(
        run(&mut rig, &plan, &mut journal, StopWhen::Immediate),
        Err(ExecutorError::Stopping)
    );
    assert_eq!(journal.audit_pending, 200);
    assert!(journal.issued.is_empty() && journal.outcomes.is_empty() && rig.lines().is_empty());
    assert!(journal.settled.is_none());
}
#[test]
fn stop_between_audit_batches_leaves_the_remaining_outbox_and_issues_nothing() {
    let input = two_actions();
    let plan = ready(&input);
    let mut rig = Rig::new(&input, [9; 32], None);
    let mut journal = FakeJournal::for_plan(&plan, rig.log.clone());
    journal.audit_pending = 200;
    assert_eq!(
        run(&mut rig, &plan, &mut journal, StopWhen::AfterAudit),
        Err(ExecutorError::Stopping)
    );
    assert_eq!(journal.audit_pending, 136);
    assert!(journal.issued.is_empty() && journal.outcomes.is_empty() && journal.settled.is_none());
    assert_eq!(rig.lines(), vec!["audit.flush"]);
}
#[test]
fn stopping_after_apply_persists_its_real_receipt_and_reopen_resumes_only_unissued_jobs() {
    for mode in [BackendMode::Commit, BackendMode::Reject] {
        let fixture = Fixture::new();
        let ledger = MemoryAuditLedger::new();
        let mut store = fixture.init();
        seed(&mut store, &ledger, 2);
        let mut input = input(&store, 2);
        for index in 0..2 {
            input.mirror.entries.insert(path(index), present(2));
        }
        let plan = ready(&input);
        store
            .reserve(store.view().unwrap().version, input.clone())
            .unwrap();
        let keys: Vec<_> = store
            .retry_pending(&input)
            .unwrap()
            .iter()
            .map(|row| row.external_key.clone())
            .collect();
        let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
        rig.backend.mode = mode;
        let mut journal = StateJournal::new(&mut store, &ledger, input.clone());
        assert_eq!(
            run(&mut rig, &plan, &mut journal, StopWhen::AfterApply),
            Err(ExecutorError::Stopping)
        );
        let rows = store.retry_pending(&input).unwrap();
        assert!(matches!(rows[0].status, ActionStatus::Known(_)));
        assert_eq!(rows[1].status, ActionStatus::Reserved);
        assert_eq!(store.view().unwrap().baseline.version, 1);
        assert_eq!(
            store.view().unwrap().pending_sequence,
            Some(plan.key().sequence)
        );
        drop(store);
        let mut store = fixture.reopen();
        assert_eq!(
            store
                .retry_pending(&input)
                .unwrap()
                .iter()
                .map(|row| row.external_key.clone())
                .collect::<Vec<_>>(),
            keys
        );
        let mut rig = Rig::new(&input, store.observation_epoch(), None);
        let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
        assert_eq!(report.already_known, 1);
        assert_eq!(report.issued, 1);
        assert_eq!(rig.backend.staged.len(), 1);
        assert!(report.settled);
    }
}
#[test]
fn stopping_after_unknown_never_fabricates_a_receipt_or_replays_the_original_job() {
    let input = two_actions();
    let plan = ready(&input);
    let mut rig = Rig::new(&input, [9; 32], None);
    rig.backend.mode = BackendMode::Unknown;
    let mut journal = FakeJournal::for_plan(&plan, rig.log.clone());
    assert_eq!(
        run(&mut rig, &plan, &mut journal, StopWhen::AfterApply),
        Err(ExecutorError::Stopping)
    );
    assert_eq!(journal.jobs[0].state, JobState::Issued);
    assert_eq!(journal.jobs[1].state, JobState::Reserved);
    assert!(journal.outcomes.is_empty() && journal.settled.is_none());
    let key = journal.jobs[0].external_key.clone();
    let mut rig = Rig::new(&input, [9; 32], None);
    let report = rig.run(&plan, &mut journal).unwrap();
    assert_eq!(report.issued, 1);
    assert_eq!(report.unknown, 1);
    assert!(!report.settled);
    assert_eq!(journal.jobs[0].external_key, key);
    assert_eq!(journal.jobs[0].state, JobState::Issued);
}
