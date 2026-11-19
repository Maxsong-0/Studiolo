//! 作业结论只看账本事实和固定阈值：卡住、逾期、从没成功、最近失败、时钟不对各有正反用例。
use super::*;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// 固定配置：最小间隔 1 小时（逾期线 2 小时），保留期 1 天；从 0 时刻开始服务。
fn jobs(now_ms: i64) -> BackupJobs {
    BackupJobs {
        now_ms,
        serving_since_ms: Some(0),
        first_attempt_ms: None,
        last_success: None,
        last_failure: None,
        latest_failed: false,
        outstanding: None,
        stuck_after_ms: DAY.unsigned_abs(),
        overdue_after_ms: (2 * HOUR).unsigned_abs(),
        recovery_point: None,
        recovery_point_budget_ms: (3 * HOUR).unsigned_abs(),
    }
}

fn success(sequence: u64, at_ms: i64) -> Option<JobMark> {
    Some(JobMark { sequence, at_ms })
}

fn open(sequence: u64, since_ms: i64, admitted: bool) -> Option<OpenJob> {
    Some(OpenJob {
        sequence,
        since_ms,
        admitted,
    })
}

#[test]
fn a_recent_success_without_a_stuck_job_is_healthy() {
    let mut facts = jobs(10 * HOUR);
    facts.last_success = success(3, 9 * HOUR);
    assert_eq!(facts.verdict(), JobsVerdict::Healthy);
    assert_eq!(facts.stuck(), Some(false));
    assert_eq!(facts.overdue(), Some(false));
    // 在做的作业还没挂过保留期：照样正常。
    facts.outstanding = open(4, 10 * HOUR - 5 * MINUTE, true);
    assert_eq!(facts.verdict(), JobsVerdict::Healthy);
    assert_eq!(
        facts.outstanding_for_ms(),
        Some((5 * MINUTE).unsigned_abs())
    );
}

#[test]
fn a_job_held_longer_than_the_retention_is_stuck_and_exactly_at_it_is_not() {
    let mut facts = jobs(3 * DAY);
    facts.last_success = success(1, 3 * DAY - HOUR);
    facts.outstanding = open(2, 2 * DAY, true);
    // 正好等于保留期：还不算卡住。
    assert_eq!(facts.stuck(), Some(false));
    assert_eq!(facts.verdict(), JobsVerdict::Healthy);
    facts.now_ms += 1;
    assert_eq!(facts.stuck(), Some(true));
    assert_eq!(facts.verdict(), JobsVerdict::Stuck);
    // 没封存成、失败结果也没补上的原意图，挂久了一样算卡住。
    facts.outstanding = open(2, 2 * DAY, false);
    assert_eq!(facts.verdict(), JobsVerdict::Stuck);
}

#[test]
fn no_success_for_more_than_twice_the_interval_is_overdue() {
    let mut facts = jobs(10 * HOUR);
    facts.last_success = success(5, 8 * HOUR);
    assert_eq!(facts.overdue(), Some(false), "正好两倍间隔还不算逾期");
    facts.now_ms += 1;
    assert_eq!(facts.overdue(), Some(true));
    assert_eq!(facts.verdict(), JobsVerdict::Overdue);
    // 卡住比逾期更该先看。
    facts.outstanding = open(6, facts.now_ms - 2 * DAY, true);
    assert_eq!(facts.verdict(), JobsVerdict::Stuck);
}

#[test]
fn never_succeeded_counts_from_the_earlier_of_serving_and_the_first_attempt() {
    // 开始服务一小时、一次都没成功：还没到逾期线，但也不能说正常。
    let mut facts = jobs(HOUR);
    assert_eq!(facts.overdue_since_ms(), Some(0));
    assert_eq!(facts.verdict(), JobsVerdict::NoSuccessYet);
    // 超过两倍间隔还是没有成功：逾期。
    facts.now_ms = 2 * HOUR + 1;
    assert_eq!(facts.verdict(), JobsVerdict::Overdue);
    // 刚重启，但账本里第一次尝试在一天前：从那时起算，重启不清零。
    let mut restarted = jobs(DAY + HOUR);
    restarted.serving_since_ms = Some(DAY + HOUR - MINUTE);
    restarted.first_attempt_ms = Some(HOUR);
    assert_eq!(restarted.overdue_since_ms(), Some(HOUR));
    assert_eq!(restarted.verdict(), JobsVerdict::Overdue);
    // 开始服务的时刻读不到、账本里也没有尝试：判断不了，不当正常。
    restarted.serving_since_ms = None;
    restarted.first_attempt_ms = None;
    assert_eq!(restarted.overdue(), None);
    assert_eq!(restarted.verdict(), JobsVerdict::ClockBehind);
}

#[test]
fn the_latest_closed_job_failing_needs_attention_until_a_later_success() {
    let mut facts = jobs(10 * HOUR);
    facts.last_success = success(1, 9 * HOUR);
    facts.last_failure = Some(FailureMark {
        sequence: 2,
        at_ms: 9 * HOUR + MINUTE,
        failure: JobFailure::Failed(FailureReason::OutOfSpace),
    });
    facts.latest_failed = true;
    assert_eq!(facts.verdict(), JobsVerdict::LatestFailed);
    // 之后又成功了：失败留着给人看，结论回到正常。
    facts.last_success = success(3, 9 * HOUR + 30 * MINUTE);
    facts.latest_failed = false;
    assert_eq!(facts.verdict(), JobsVerdict::Healthy);
    assert!(facts.last_failure.is_some());
}

#[test]
fn a_clock_behind_the_ledger_is_unknown_not_healthy() {
    let mut facts = jobs(HOUR);
    facts.last_success = success(1, 2 * HOUR);
    assert_eq!(facts.overdue(), None);
    assert_eq!(facts.verdict(), JobsVerdict::ClockBehind);
    // 原意图落在将来：卡没卡住也判断不了。
    let mut held = jobs(HOUR);
    held.last_success = success(1, HOUR - MINUTE);
    held.outstanding = open(2, 2 * HOUR, true);
    assert_eq!(held.stuck(), None);
    assert_eq!(held.verdict(), JobsVerdict::ClockBehind);
    // 已知的失败照样排在判断不了前面。
    held.latest_failed = true;
    assert_eq!(held.verdict(), JobsVerdict::LatestFailed);
}
