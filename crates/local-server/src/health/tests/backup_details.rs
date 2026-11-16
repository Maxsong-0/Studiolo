//! 「备份作业」一节和明细出自同一份账本事实：正常、卡住、逾期、从没成功、最近失败、判断不了各有用例。
use super::{HealthBody, complete, healthy_jobs};
use crate::backup::{BackupJobsEvidence, FailureMark, JobFailure, OpenJob};
use serde_json::Value;
use studiolo_backup::worker::{restic::UnverifiedReason, state::FailureReason};

const NOW: i64 = 100_000;
const NOW_MS: i64 = NOW * 1_000;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

fn body(evidence: BackupJobsEvidence) -> Value {
    let mut snapshot = complete(NOW);
    snapshot.backup_jobs = evidence;
    serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap()
}

fn section(body: &Value) -> &Value {
    body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["code"] == "backup-jobs")
        .expect("必须包含备份作业一节")
}

fn read(change: impl FnOnce(&mut crate::backup::BackupJobs)) -> Value {
    let mut jobs = healthy_jobs(NOW);
    change(&mut jobs);
    body(BackupJobsEvidence::Read(jobs))
}

#[test]
fn a_recent_success_is_normal_and_the_details_are_closed() {
    let body = read(|_| {});
    let jobs = section(&body);
    assert_eq!(jobs["state"], "normal");
    assert_eq!(jobs["detail"], "最近一次成功是 1 分钟前的第 4 号");
    let backup = &body["backup"];
    assert_eq!(backup.as_object().unwrap().len(), 11);
    assert_eq!(backup["state"], "normal");
    assert_eq!(backup["verdict"], "healthy");
    assert_eq!(backup["last_success"]["sequence"], "4");
    assert_eq!(backup["last_success"]["at"], "1970-01-02T03:45:40Z");
    assert_eq!(backup["as_of"], "1970-01-02T03:46:40Z");
    assert_eq!(backup["overdue"], false);
    assert!(backup["outstanding"].is_null());
    assert!(backup["last_failure"].is_null());
    assert_eq!(backup["stuck_after_seconds"], "86400");
    assert_eq!(backup["overdue_after_seconds"], "7200");
    assert_eq!(body["overall"], "normal", "其余各节齐备时整体可以是绿");
}

#[test]
fn a_job_held_past_the_retention_is_stuck_and_degrades_the_page() {
    let body = read(|jobs| {
        jobs.outstanding = Some(OpenJob {
            sequence: 5,
            since_ms: NOW_MS - DAY - 2 * HOUR,
            admitted: true,
        });
    });
    let jobs = section(&body);
    assert_eq!(jobs["state"], "degraded");
    assert_eq!(
        jobs["detail"],
        "第 5 号作业挂了 1 天 2 小时还没收口，超过保留期 1 天，算卡住"
    );
    assert_eq!(body["backup"]["verdict"], "stuck");
    assert_eq!(body["backup"]["outstanding"]["sequence"], "5");
    assert_eq!(body["backup"]["outstanding"]["admitted"], true);
    assert_eq!(body["backup"]["outstanding"]["stuck"], true);
    assert_eq!(body["overall"], "degraded");
    // 反例：在做、没过保留期，照样正常，明细里看得到它挂了多久。
    let body = read(|jobs| {
        jobs.outstanding = Some(OpenJob {
            sequence: 5,
            since_ms: NOW_MS - 3 * MINUTE,
            admitted: false,
        });
    });
    assert_eq!(section(&body)["state"], "normal");
    assert_eq!(
        section(&body)["detail"],
        "最近一次成功是 1 分钟前的第 4 号；第 5 号作业在做，已挂 3 分钟"
    );
    assert_eq!(body["backup"]["outstanding"]["stuck"], false);
}

#[test]
fn no_success_for_more_than_twice_the_interval_is_overdue() {
    let body = read(|jobs| {
        jobs.last_success.as_mut().unwrap().at_ms = NOW_MS - 3 * HOUR;
    });
    assert_eq!(section(&body)["state"], "degraded");
    assert_eq!(
        section(&body)["detail"],
        "最近一次成功是 3 小时前的第 4 号，超过两倍最小间隔 2 小时，算逾期"
    );
    assert_eq!(body["backup"]["verdict"], "overdue");
    assert_eq!(body["backup"]["overdue"], true);
    assert_eq!(body["backup"]["overdue_since"], "1970-01-02T00:46:40Z");
}

#[test]
fn never_succeeded_is_unknown_until_the_line_and_overdue_after_it() {
    let body = read(|jobs| {
        jobs.last_success = None;
        jobs.serving_since_ms = Some(NOW_MS - HOUR);
        jobs.first_attempt_ms = None;
    });
    assert_eq!(section(&body)["state"], "unknown");
    assert_eq!(
        section(&body)["detail"],
        "还没有成功的备份，离逾期线还有 1 小时"
    );
    assert_eq!(body["backup"]["verdict"], "no_success_yet");
    assert!(body["backup"]["last_success"].is_null());

    let body = read(|jobs| {
        jobs.last_success = None;
        jobs.serving_since_ms = Some(NOW_MS - HOUR);
        jobs.first_attempt_ms = Some(NOW_MS - 2 * HOUR - 5 * MINUTE);
    });
    assert_eq!(section(&body)["state"], "degraded");
    assert_eq!(
        section(&body)["detail"],
        "从开始服务或第一次尝试算起已经 2 小时 5 分钟，一次成功都没有，超过两倍最小间隔 2 小时，算逾期"
    );
    assert_eq!(body["backup"]["verdict"], "overdue");
}

#[test]
fn the_latest_failure_is_shown_with_its_fixed_reason() {
    let body = read(|jobs| {
        jobs.last_failure = Some(FailureMark {
            sequence: 5,
            at_ms: NOW_MS - 30_000,
            failure: JobFailure::Failed(FailureReason::OutOfSpace),
        });
        jobs.latest_failed = true;
    });
    assert_eq!(section(&body)["state"], "degraded");
    assert_eq!(
        section(&body)["detail"],
        "最近收口的第 5 号作业没成功：磁盘空间不够；按批准的间隔重试"
    );
    let failure = &body["backup"]["last_failure"];
    assert_eq!(failure["result"], "failed");
    assert_eq!(failure["reason"], "out_of_space");
    assert_eq!(body["backup"]["latest_failed"], true);

    // 之后又成功了：失败还在明细里，结论回到正常。
    let body = read(|jobs| {
        jobs.last_failure = Some(FailureMark {
            sequence: 3,
            at_ms: NOW_MS - 30 * MINUTE,
            failure: JobFailure::Unverified(UnverifiedReason::TargetUnreachable),
        });
    });
    assert_eq!(section(&body)["state"], "normal");
    assert_eq!(body["backup"]["last_failure"]["result"], "unverified");
    assert_eq!(
        body["backup"]["last_failure"]["reason"],
        "target_unreachable"
    );

    let body = read(|jobs| {
        jobs.last_failure = Some(FailureMark {
            sequence: 6,
            at_ms: NOW_MS - 10_000,
            failure: JobFailure::Unrecognized,
        });
        jobs.latest_failed = true;
    });
    assert_eq!(
        section(&body)["detail"],
        "最近收口的第 6 号作业没成功：结果读不出来，不当成功；按批准的间隔重试"
    );
    assert_eq!(body["backup"]["last_failure"]["result"], "unrecognized");
    assert!(body["backup"]["last_failure"]["reason"].is_null());
}

#[test]
fn a_clock_behind_the_ledger_or_missing_evidence_is_unknown_not_normal() {
    let behind = read(|jobs| {
        jobs.last_success.as_mut().unwrap().at_ms = NOW_MS + MINUTE;
    });
    assert_eq!(section(&behind)["state"], "unknown");
    assert_eq!(behind["backup"]["verdict"], "clock_behind");
    assert!(behind["backup"]["overdue"].is_null());
    assert_ne!(behind["overall"], "normal");

    for (evidence, prefix) in [
        (BackupJobsEvidence::Disabled, "备份生产者没有启用"),
        (
            BackupJobsEvidence::Unreadable,
            "这一次没读到账本里的备份事件",
        ),
    ] {
        let body = body(evidence);
        assert_eq!(section(&body)["state"], "unknown");
        assert!(
            section(&body)["detail"]
                .as_str()
                .unwrap()
                .starts_with(prefix)
        );
        assert!(body["backup"].is_null());
        assert_eq!(body["overall"], "unknown");
        assert_eq!(body["evidence_complete"], false);
    }
}

#[test]
fn backup_details_never_carry_paths_or_identities() {
    let long_ago = read(|jobs| {
        jobs.now_ms = i64::MAX;
        jobs.outstanding = Some(OpenJob {
            sequence: u64::MAX,
            since_ms: 1,
            admitted: false,
        });
    });
    let detail = section(&long_ago)["detail"].as_str().unwrap();
    assert!(detail.starts_with(&format!("第 {} 号作业挂了", u64::MAX)));
    for text in [detail.to_owned(), long_ago["backup"].to_string()] {
        assert!(!text.contains('/') && !text.contains('\\') && !text.contains('@'));
    }
    assert!(long_ago["backup"]["as_of"].is_null(), "越界的时刻不编造");
}
