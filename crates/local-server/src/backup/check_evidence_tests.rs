//! 抽查结论：已知的问题排在判断不了前面；首轮里读了几组都算正常，首轮之后每组 32 天内都要读过。
use super::*;

const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;
const NOW: i64 = 1_800_000_000_000;

fn facts() -> RepositoryChecks {
    RepositoryChecks {
        now_ms: NOW,
        anchor_ms: Some(NOW - 2 * HOUR),
        repository_since_ms: None,
        latest_record_ms: None,
        running: None,
        latest: None,
        last_clean_ms: [None; BUCKETS],
        problems: Vec::new(),
    }
}

/// 当前仓库第一次有结果在 `since_days` 天前，前 `count` 组都在 `age_days` 天前读干净。
fn read(since_days: i64, count: usize, age_days: i64) -> RepositoryChecks {
    let mut facts = facts();
    facts.repository_since_ms = Some(NOW - since_days * DAY);
    facts.latest_record_ms = Some(NOW - HOUR);
    for slot in facts.last_clean_ms.iter_mut().take(count) {
        *slot = Some(NOW - age_days * DAY);
    }
    facts.last_clean_ms[0] = Some(NOW - HOUR);
    facts
}

#[test]
fn a_problem_degrades_before_anything_else_even_with_a_clock_behind() {
    let mut facts = read(40, 30, 1);
    facts.latest_record_ms = Some(NOW + HOUR);
    facts.problems.push(CheckProblem {
        bucket: 4,
        at_ms: NOW + HOUR,
        errors: 1,
        broken_packs: 0,
        times: 1,
    });
    assert_eq!(facts.verdict(), CheckVerdict::Problem);
}

#[test]
fn nothing_clean_yet_is_unknown_for_72_hours_then_stale() {
    let mut facts = facts();
    assert_eq!(facts.verdict(), CheckVerdict::NotYet);
    facts.anchor_ms = Some(NOW - 72 * HOUR);
    assert_eq!(facts.verdict(), CheckVerdict::NotYet);
    facts.anchor_ms = Some(NOW - 72 * HOUR - 1);
    assert_eq!(facts.verdict(), CheckVerdict::Stale);
    facts.anchor_ms = None;
    assert_eq!(facts.verdict(), CheckVerdict::ClockBehind);
}

#[test]
fn the_last_clean_check_must_be_within_72_hours() {
    let mut facts = read(5, 3, 1);
    facts.last_clean_ms[0] = Some(NOW - 73 * HOUR);
    facts.last_clean_ms[1] = Some(NOW - 80 * HOUR);
    facts.last_clean_ms[2] = None;
    assert_eq!(facts.verdict(), CheckVerdict::Stale);
    facts.last_clean_ms[2] = Some(NOW - 71 * HOUR);
    assert_eq!(facts.verdict(), CheckVerdict::Healthy);
}

#[test]
fn the_first_cycle_reads_as_healthy_with_partial_coverage() {
    let facts = read(10, 10, 5);
    assert_eq!(facts.first_cycle_done(), Some(false));
    assert_eq!(facts.fresh_buckets(), 10);
    assert_eq!(facts.verdict(), CheckVerdict::Healthy);
}

#[test]
fn after_the_first_cycle_every_bucket_must_have_been_read_within_32_days() {
    let mut facts = read(40, 30, 20);
    assert_eq!(facts.verdict(), CheckVerdict::Healthy);
    assert_eq!(facts.fresh_buckets(), 30);
    assert_eq!(facts.oldest_clean_ms(), Some(20 * 24 * 3_600_000));
    facts.last_clean_ms[6] = Some(NOW - 32 * DAY - 1);
    facts.last_clean_ms[9] = None;
    assert_eq!(facts.verdict(), CheckVerdict::CoverageOverdue);
    assert_eq!(facts.overdue_buckets(), vec![7, 10]);
    assert_eq!(facts.fresh_buckets(), 28);
}

#[test]
fn a_clock_behind_the_ledger_cannot_judge_freshness_or_coverage() {
    let mut facts = read(40, 30, 1);
    facts.latest_record_ms = Some(NOW + 1);
    assert_eq!(facts.verdict(), CheckVerdict::ClockBehind);
}

#[test]
fn a_clock_behind_the_ledger_still_reports_what_it_can_prove() {
    // 本地时钟早于账本时算出的间隔只会偏短：上次干净已经算出超过 72 小时，就一定过期。
    let mut facts = read(5, 3, 1);
    facts.last_clean_ms[0] = Some(NOW - 73 * HOUR);
    facts.last_clean_ms[1] = Some(NOW - 80 * HOUR);
    facts.last_clean_ms[2] = None;
    facts.latest_record_ms = Some(NOW + HOUR);
    assert_eq!(facts.verdict(), CheckVerdict::Stale);
    // 上次干净的时刻本身晚于本地时钟：过期判断不了。
    facts.last_clean_ms[2] = Some(NOW + HOUR);
    assert_eq!(facts.verdict(), CheckVerdict::ClockBehind);

    // 首轮早就读完、有组从没读干净：覆盖逾期照样成立；读干净的时刻晚于本地时钟的那组不算逾期。
    let mut facts = read(40, 30, 1);
    facts.latest_record_ms = Some(NOW + HOUR);
    facts.last_clean_ms[4] = Some(NOW + HOUR);
    assert_eq!(facts.verdict(), CheckVerdict::ClockBehind);
    assert!(facts.overdue_buckets().is_empty());
    facts.last_clean_ms[9] = None;
    assert_eq!(facts.overdue_buckets(), vec![10]);
    assert_eq!(facts.verdict(), CheckVerdict::CoverageOverdue);
}

#[test]
fn the_text_thresholds_are_the_judgement_thresholds() {
    assert_eq!(RepositoryChecks::FRESHNESS_HOURS * HOUR, FRESHNESS_MS);
    assert_eq!(RepositoryChecks::COVERAGE_DAYS * DAY, COVERAGE_MS);
    let mut facts = facts();
    facts.anchor_ms = Some(NOW - 2 * HOUR);
    assert_eq!(facts.freshness_left_ms(), Some(70 * 3_600_000));
    facts.anchor_ms = Some(NOW + 1);
    assert_eq!(facts.freshness_left_ms(), None);
}
