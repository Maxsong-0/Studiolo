//! 失败契约：每一条都必须在发出第一条删除之前整批阻断，且不留下半截状态。

use super::fake_journal::FakeJournal;
use super::ports::*;
use super::runner::*;
use super::support::*;
use crate::executor::*;
use crate::*;

pub(super) fn drive(plan: &Plan) -> (Result<ExecutionReport, ExecutorError>, FakeJournal, Rig) {
    let log = log();
    let mut journal = FakeJournal::for_plan(plan, log);
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], Some(present(1)));
    let result = rig.run(plan, &mut journal);
    (result, journal, rig)
}

#[test]
fn requesting_a_deletion_without_quiescence_evidence_issues_nothing_and_says_so() {
    let plan = forged_deletions(200, 5, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    rig.quiescence = FakeQuiescence::refusing(QuiescenceFault::Unavailable);
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::DeletionWithoutQuiescence)
    );
    assert!(journal.issued.is_empty());
    assert_eq!(rig.cas.usage().entries, 0);
}

#[test]
fn unsupported_unknown_or_open_spawn_gates_are_never_downgraded_into_quiescence() {
    for (evidence, expected) in [
        (
            QuiescenceEvidence::Unsupported,
            QuiescenceFault::Unsupported,
        ),
        (QuiescenceEvidence::Unknown, QuiescenceFault::Unknown),
        (
            QuiescenceEvidence::ControlledSubgroupIdle {
                spawn_gate_closed: false,
            },
            QuiescenceFault::SpawnGateOpen,
        ),
    ] {
        let plan = forged_deletions(200, 0, true);
        let log = log();
        let mut journal = FakeJournal::for_plan(&plan, log);
        let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
        rig.quiescence.evidence = evidence;
        assert_eq!(
            rig.run(&plan, &mut journal),
            Err(ExecutorError::QuiescenceMissing(expected))
        );
    }
}

#[test]
fn a_quiescence_proof_older_than_the_frozen_observation_is_refused() {
    let plan = forged_deletions(200, 0, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    rig.quiescence.observation = plan.frozen_input().mirror.sequence - 1;
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::QuiescenceMissing(QuiescenceFault::Stale))
    );
}

#[test]
fn an_incomplete_observation_blocks_the_whole_batch_before_any_transfer() {
    let mut input = raw_input(20, 1, true);
    input.mirror.complete = false;
    let plan = forge(
        input,
        Vec::new(),
        Vec::new(),
        DeleteStats {
            baseline_live_paths: 20,
            ..DeleteStats::default()
        },
    );
    let (result, journal, rig) = drive(&plan);
    assert_eq!(result, Err(ExecutorError::IncompleteObservation));
    assert!(journal.issued.is_empty());
    assert_eq!(rig.first("open"), None);
}

#[test]
fn a_deletion_without_a_surviving_common_baseline_entry_is_refused_as_incomplete() {
    let mut input = raw_input(20, 1, true);
    let sequence = input.plan_sequence;
    let action = mirror_delete(&mut input, 0, sequence);
    input.baseline.entries.remove(&path(0));
    let plan = forge(
        input,
        vec![action],
        Vec::new(),
        DeleteStats {
            mirror: 1,
            baseline_live_paths: 19,
            ..DeleteStats::default()
        },
    );
    let (result, journal, _) = drive(&plan);
    assert_eq!(result, Err(ExecutorError::IncompleteObservation));
    assert!(journal.issued.is_empty());
}

#[test]
fn a_conflict_path_may_never_carry_an_executable_action() {
    let mut input = raw_input(20, 1, true);
    let sequence = input.plan_sequence;
    let action = mirror_delete(&mut input, 0, sequence);
    let conflict = Conflict {
        path: path(0),
        reason: ConflictReason::DeleteAgainstModification,
        baseline: input.baseline.entries.get(&path(0)).cloned(),
        observed: Pair {
            canonical: Presence::Missing,
            mirror: present(2),
        },
    };
    let plan = forge(
        input,
        vec![action],
        vec![conflict],
        DeleteStats {
            mirror: 1,
            baseline_live_paths: 20,
            ..DeleteStats::default()
        },
    );
    let (result, journal, _) = drive(&plan);
    assert_eq!(result, Err(ExecutorError::ConflictHasAction));
    assert!(journal.issued.is_empty());
}

#[test]
fn a_plan_whose_frozen_statistics_disagree_with_the_recount_is_refused() {
    let mut plan = forged_deletions(20, 2, true);
    plan.stats.mirror = 1;
    let (result, journal, _) = drive(&plan);
    assert_eq!(result, Err(ExecutorError::PlanMismatch));
    assert!(journal.issued.is_empty());
}

#[test]
fn a_lost_audit_acknowledgement_stops_the_batch_before_any_issue() {
    let plan = forged_deletions(200, 3, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    journal.audit_fault = Some(JournalFault::AuditUnavailable);
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::Journal(JournalFault::AuditUnavailable))
    );
    assert!(journal.issued.is_empty());
    assert_eq!(rig.cas.usage().entries, 0);
}

#[test]
fn an_audit_backlog_that_never_advances_fails_instead_of_looping_forever() {
    let plan = forged_deletions(200, 1, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    journal.audit_stalled = true;
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::AuditStalled)
    );
    assert!(journal.issued.is_empty());
}

#[test]
fn a_persistent_fuse_or_a_retired_sequence_refuses_the_batch_outright() {
    let plan = forged_deletions(200, 1, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log.clone());
    journal.fused = true;
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    assert_eq!(rig.run(&plan, &mut journal), Err(ExecutorError::Fused));

    let mut journal = FakeJournal::for_plan(&plan, log);
    journal.retired_through = plan.key().sequence;
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    assert_eq!(rig.run(&plan, &mut journal), Err(ExecutorError::Retired));
    assert!(journal.issued.is_empty());
}

#[test]
fn content_that_does_not_match_the_frozen_address_is_never_issued_or_staged() {
    for (mode, expected) in [
        (
            ContentMode::Corrupt,
            ExecutorError::Cas(CasFault::AddressMismatch),
        ),
        (
            ContentMode::Truncate,
            ExecutorError::Cas(CasFault::DeclaredBytesMismatch),
        ),
        (
            ContentMode::Unavailable,
            ExecutorError::Transfer(TransferFault::Unavailable),
        ),
    ] {
        let plan = forged_deletions(200, 1, true);
        let log = log();
        let mut journal = FakeJournal::for_plan(&plan, log);
        let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
        rig.content.mode = mode;
        assert_eq!(rig.run(&plan, &mut journal), Err(expected));
        assert!(journal.issued.is_empty());
        assert_eq!(rig.cas.usage().entries, 0);
    }
}

#[test]
fn a_witness_that_cannot_recheck_stops_before_the_action_is_issued() {
    let plan = forged_deletions(200, 1, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    rig.witness.fault = Some(WitnessFault::Stale);
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::Witness(WitnessFault::Stale))
    );
    assert!(journal.issued.is_empty());
}

#[test]
fn an_action_whose_intent_is_not_acknowledged_is_never_issued() {
    let plan = forged_deletions(200, 1, true);
    let log = log();
    let mut journal = FakeJournal::for_plan(&plan, log);
    for job in &mut journal.jobs {
        job.state = JobState::IntentPending;
    }
    let mut rig = Rig::new(plan.frozen_input(), [0; 32], None);
    assert_eq!(
        rig.run(&plan, &mut journal),
        Err(ExecutorError::Journal(JournalFault::IntentRequired))
    );
    assert!(journal.issued.is_empty());
}
