//! 抽查节奏只看账本：轮转每 24 小时往下一组走，不干净的组另走重查，坏组不卡住整圈。
use super::*;
use studiolo_audit_ledger::{AuditEvent, IntegrityHash};
use studiolo_backup::worker::{
    repository_check::{
        CheckAssignment, CheckOutcome, IncompleteReason, check_abandoned, check_intent,
        check_outcome,
    },
    restic::RepositoryRef,
};

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const START: i64 = 1_800_000_000_000;

fn bucket(value: u8) -> CheckBucket {
    CheckBucket::new(value).unwrap()
}

fn key(sequence: u64, value: u8) -> CheckKey {
    CheckKey {
        sequence: NonZeroU64::new(sequence).unwrap(),
        bucket: bucket(value),
    }
}

fn digest(name: &str) -> RepositoryDigest {
    RepositoryDigest::of(
        &RepositoryRef::parse(format!("rest:https://{name}.example.test/repo/")).unwrap(),
    )
}

/// 按账本顺序喂事件；时刻由用例给，不依赖真实时钟。
struct Ledger {
    checks: Checks,
    offset: u64,
}

impl Ledger {
    fn new() -> Self {
        Self {
            checks: Checks::default(),
            offset: 0,
        }
    }
    fn feed(&mut self, at_ms: i64, event: AuditEvent) {
        self.offset += 1;
        let record = AuditRecord::from_parts(
            self.offset,
            jiff::Timestamp::from_millisecond(at_ms).unwrap(),
            IntegrityHash::genesis(),
            IntegrityHash::genesis(),
            event,
        );
        self.checks.absorb(TARGET, &record);
    }
    fn intent(&mut self, at_ms: i64, key: CheckKey, purpose: CheckPurpose) {
        self.feed(
            at_ms,
            check_intent(&CheckAssignment { key, purpose }, TARGET).unwrap(),
        );
    }
    fn outcome(&mut self, at_ms: i64, key: CheckKey, outcome: CheckOutcome) {
        self.feed(
            at_ms,
            check_outcome(&key, &outcome, &CheckRecorder::Worker(digest("nas"))).unwrap(),
        );
    }
    /// 生产者照 `due` 派一次检查、worker 跑完交结果，返回派下的组号和原因。
    fn run(&mut self, at_ms: i64, outcome: CheckOutcome) -> (CheckBucket, CheckPurpose) {
        let (bucket, purpose) = self.checks.due(at_ms).expect("应当到期");
        let key = CheckKey {
            sequence: self.checks.next_sequence().unwrap(),
            bucket,
        };
        self.intent(at_ms, key, purpose);
        self.outcome(at_ms + 5 * MINUTE, key, outcome);
        (bucket, purpose)
    }
}

fn problem() -> CheckOutcome {
    CheckOutcome::problem(1, 0).unwrap()
}

#[test]
fn the_first_check_is_due_at_once_and_reads_bucket_one() {
    let ledger = Ledger::new();
    assert_eq!(
        ledger.checks.due(START),
        Some((CheckBucket::FIRST, CheckPurpose::Rotation))
    );
    assert_eq!(ledger.checks.next_sequence().unwrap().get(), 1);
    assert_eq!(ledger.checks.running(START), None);
    assert!(ledger.checks.abandoned(START).is_empty());
}

#[test]
fn rotation_advances_once_a_day_from_the_intent_and_wraps_after_thirty() {
    let mut ledger = Ledger::new();
    let mut at = START;
    for expected in (1..=30).chain(1..=2) {
        assert_eq!(
            ledger.run(at, CheckOutcome::clean()),
            (bucket(expected), CheckPurpose::Rotation)
        );
        // 干净之后到下一次轮转之前都不到期。
        assert_eq!(ledger.checks.due(at + 23 * HOUR), None);
        at += DAY;
    }
    assert_eq!(ledger.checks.next_sequence().unwrap().get(), 33);
}

#[test]
fn an_open_intent_is_running_for_thirty_minutes_then_abandoned() {
    let mut ledger = Ledger::new();
    ledger.intent(START, key(1, 1), CheckPurpose::Rotation);
    assert_eq!(
        ledger.checks.running(START + 29 * MINUTE).unwrap().key,
        key(1, 1)
    );
    assert!(ledger.checks.abandoned(START + 29 * MINUTE).is_empty());
    assert_eq!(ledger.checks.running(START + 30 * MINUTE), None);
    assert_eq!(
        ledger.checks.abandoned(START + 30 * MINUTE),
        vec![key(1, 1)]
    );
    // 生产者补的「没做完」收口这条意图，这一组一小时后重查。
    ledger.feed(START + 31 * MINUTE, check_abandoned(&key(1, 1)).unwrap());
    assert!(ledger.checks.abandoned(START + DAY).is_empty());
    assert_eq!(ledger.checks.due(START + 31 * MINUTE + 59 * MINUTE), None);
    assert_eq!(
        ledger.checks.due(START + 31 * MINUTE + HOUR),
        Some((bucket(1), CheckPurpose::Retry))
    );
}

#[test]
fn a_failing_bucket_is_retried_after_an_hour_then_daily_while_rotation_moves_on() {
    let mut ledger = Ledger::new();
    ledger.run(START, CheckOutcome::clean());
    let day2 = START + DAY;
    assert_eq!(
        ledger.run(day2, problem()),
        (bucket(2), CheckPurpose::Rotation)
    );
    let failed_at = day2 + 5 * MINUTE;
    assert_eq!(ledger.checks.due(failed_at + 59 * MINUTE), None);
    assert_eq!(
        ledger.run(failed_at + HOUR, problem()),
        (bucket(2), CheckPurpose::Retry)
    );
    // 连续两次不干净：下一次重查等 24 小时；轮转照常在第二天往第 3 组走。
    let second = failed_at + HOUR + 5 * MINUTE;
    assert_eq!(ledger.checks.due(second + 2 * HOUR), None);
    assert_eq!(
        ledger.run(day2 + DAY, CheckOutcome::clean()),
        (bucket(3), CheckPurpose::Rotation)
    );
    assert_eq!(ledger.checks.due(second + DAY - MINUTE), None);
    assert_eq!(
        ledger.checks.due(second + DAY),
        Some((bucket(2), CheckPurpose::Retry))
    );
    let state = ledger.checks.buckets()[&bucket(2)];
    assert_eq!(state.non_clean_streak, 2);
    assert_eq!(state.problem_streak, 2);
    // 修好之后重查干净：这组的连续计数清零，不再重查。
    ledger.run(second + DAY, CheckOutcome::clean());
    let state = ledger.checks.buckets()[&bucket(2)];
    assert_eq!((state.non_clean_streak, state.problem_streak), (0, 0));
    assert_eq!(state.conclusive, Some(Conclusive::Clean));
}

#[test]
fn retries_come_before_rotation_oldest_first_and_at_least_an_hour_apart() {
    let mut ledger = Ledger::new();
    // 第 1、2 组各在轮转里查出一次问题（中间那次重查不在这个用例里）。
    ledger.intent(START, key(1, 1), CheckPurpose::Rotation);
    ledger.outcome(START + 5 * MINUTE, key(1, 1), problem());
    ledger.intent(START + DAY, key(2, 2), CheckPurpose::Rotation);
    ledger.outcome(START + DAY + 5 * MINUTE, key(2, 2), problem());
    // 两组都过了一小时、轮转也到期：先重查结果最早的第 1 组。
    let later = START + 2 * DAY;
    assert_eq!(
        ledger.checks.due(later),
        Some((bucket(1), CheckPurpose::Retry))
    );
    ledger.run(later, problem());
    // 第 2 组也该重查了，但离上一次重查不到一小时：先做到期的轮转。
    assert_eq!(
        ledger.checks.due(later + 10 * MINUTE),
        Some((bucket(3), CheckPurpose::Rotation))
    );
    ledger.run(later + 10 * MINUTE, CheckOutcome::clean());
    assert_eq!(ledger.checks.due(later + 59 * MINUTE), None);
    assert_eq!(
        ledger.checks.due(later + HOUR),
        Some((bucket(2), CheckPurpose::Retry))
    );
}

#[test]
fn incomplete_results_retry_without_counting_as_problems() {
    let mut ledger = Ledger::new();
    ledger.run(START, problem());
    ledger.run(
        START + HOUR + 5 * MINUTE,
        CheckOutcome::incomplete(IncompleteReason::Preempted),
    );
    let state = ledger.checks.buckets()[&bucket(1)];
    assert_eq!(state.non_clean_streak, 2);
    assert_eq!(state.problem_streak, 1, "没做完不算也不打断查出问题");
    assert!(matches!(state.conclusive, Some(Conclusive::Problem { .. })));
    assert_eq!(state.last_clean_ms, None);
}

#[test]
fn a_clock_behind_the_ledger_never_makes_anything_due() {
    let mut ledger = Ledger::new();
    ledger.run(START, problem());
    assert_eq!(ledger.checks.due(START), None, "本地时钟早于最近的结果");
    assert_eq!(ledger.checks.due(START - DAY), None);
    // 时钟早于原意图：照样算还在查，不补「没做完」。
    ledger.intent(START + DAY, key(9, 3), CheckPurpose::Rotation);
    assert!(ledger.checks.running(START).is_some());
    assert!(ledger.checks.abandoned(START).is_empty());
}

#[test]
fn other_targets_take_sequences_but_never_open_checks() {
    let mut ledger = Ledger::new();
    ledger.feed(
        START,
        check_intent(
            &CheckAssignment {
                key: key(5, 1),
                purpose: CheckPurpose::Rotation,
            },
            BackupTargetKind::LocalTemporaryRepository,
        )
        .unwrap(),
    );
    assert_eq!(ledger.checks.next_sequence().unwrap().get(), 6);
    assert_eq!(ledger.checks.running(START), None);
    assert_eq!(
        ledger.checks.due(START),
        Some((CheckBucket::FIRST, CheckPurpose::Rotation))
    );
    // 没有本目标原意图的结果不算。
    ledger.outcome(START, key(5, 1), CheckOutcome::clean());
    assert!(ledger.checks.buckets().is_empty());
    assert_eq!(ledger.checks.latest(), None);
}

#[test]
fn a_new_repository_forgets_the_old_buckets_and_restarts_coverage() {
    let mut ledger = Ledger::new();
    ledger.run(START, CheckOutcome::clean());
    ledger.run(START + DAY, problem());
    assert_eq!(
        ledger.checks.repository_since_ms(),
        Some(START + 5 * MINUTE)
    );
    assert_eq!(ledger.checks.buckets().len(), 2);
    let moved = START + 2 * DAY;
    ledger.intent(moved, key(3, 3), CheckPurpose::Rotation);
    ledger.feed(
        moved + MINUTE,
        check_outcome(
            &key(3, 3),
            &CheckOutcome::clean(),
            &CheckRecorder::Worker(digest("other")),
        )
        .unwrap(),
    );
    assert_eq!(ledger.checks.repository_since_ms(), Some(moved + MINUTE));
    let buckets: Vec<u8> = ledger.checks.buckets().keys().map(|b| b.get()).collect();
    assert_eq!(buckets, vec![3], "旧仓库第 1、2 组的结果不再算");
}
