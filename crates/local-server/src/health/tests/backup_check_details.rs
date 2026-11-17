//! 「备份仓库检查」一节：每种结论各有用例，正文都过得了脱敏边界（不退成兜底句）。
use super::{HealthBody, complete, healthy_checks};
use crate::backup::{
    CheckMark, CheckProblem, RepositoryCheckEvidence, RepositoryChecks, RunningCheck,
};
use serde_json::Value;
use studiolo_backup::worker::repository_check::{CheckResult, IncompleteReason};

const NOW: i64 = 100_000;
const NOW_MS: i64 = NOW * 1_000;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const FALLBACK: &str = "仓库检查的结论没能写成脱敏正文，按未知处理";

fn section(evidence: RepositoryCheckEvidence) -> (String, String) {
    let mut snapshot = complete(NOW);
    snapshot.backup_checks = evidence;
    let body: Value = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    let found = body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["code"] == "backup-repository-check")
        .expect("必须包含备份仓库检查一节")
        .clone();
    assert_eq!(found["title"], "备份仓库检查");
    let detail = found["detail"].as_str().unwrap().to_owned();
    assert_ne!(detail, FALLBACK, "正文必须过得了脱敏边界");
    (found["state"].as_str().unwrap().to_owned(), detail)
}

fn read(change: impl FnOnce(&mut RepositoryChecks)) -> (String, String) {
    let mut checks = healthy_checks(NOW);
    change(&mut checks);
    section(RepositoryCheckEvidence::Read(Box::new(checks)))
}

#[test]
fn the_first_cycle_is_normal_and_says_how_many_buckets_were_read() {
    assert_eq!(
        read(|_| {}),
        (
            "normal".to_owned(),
            "最近一次干净的检查是 10 分钟前；首轮还在读，30 组里读过 1 组".to_owned()
        )
    );
    let (state, detail) = read(|checks| {
        checks.running = Some(RunningCheck {
            bucket: 2,
            since_ms: NOW_MS - MINUTE,
        });
    });
    assert_eq!(state, "normal");
    assert!(detail.ends_with("；第 2 组正在查"), "{detail}");
}

#[test]
fn after_the_first_cycle_normal_names_the_oldest_bucket() {
    let (state, detail) = read(|checks| {
        checks.repository_since_ms = Some(NOW_MS - 40 * DAY);
        for slot in &mut checks.last_clean_ms {
            *slot = Some(NOW_MS - 20 * DAY);
        }
        checks.last_clean_ms[0] = Some(NOW_MS - HOUR);
    });
    assert_eq!(state, "normal");
    assert_eq!(
        detail,
        "最近一次干净的检查是 1 小时前；32 天内 30 组都读过，最久的一组 20 天前"
    );
}

#[test]
fn only_broken_packs_are_called_damage_and_problems_stay_until_a_clean_recheck() {
    let problem = |bucket, errors, broken_packs, times| CheckProblem {
        bucket,
        at_ms: NOW_MS - 2 * HOUR,
        errors,
        broken_packs,
        times,
    };
    let (state, detail) = read(|checks| checks.problems.push(problem(5, 2, 0, 1)));
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "第 5 组 2 小时前检查没通过（报错 2 处），可能是仓库损坏，也可能是连不上或凭据不对；同一组重查干净前一直显示"
    );
    let (state, detail) = read(|checks| {
        checks.problems.push(problem(5, 2, 0, 1));
        checks.problems.push(problem(9, 3, 1, 2));
    });
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "第 9 组 2 小时前读到 1 个损坏的数据包，要用管理员身份按维护步骤修，从上次干净以来第 2 次；另有 1 组也没通过；同一组重查干净前一直显示"
    );
}

#[test]
fn no_clean_check_for_72_hours_degrades_and_says_why_the_last_one_failed() {
    let (state, detail) = read(|checks| {
        checks.last_clean_ms[0] = Some(NOW_MS - 73 * HOUR);
        checks.latest = Some(CheckMark {
            bucket: 4,
            at_ms: NOW_MS - 30 * MINUTE,
            result: CheckResult::Incomplete,
            reason: Some(IncompleteReason::Preempted),
        });
    });
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "最近一次干净的检查是 3 天 1 小时前，超过 72 小时；最近一次是 30 分钟前第 4 组，没做完（让给了备份作业）"
    );
    let (state, detail) = read(|checks| {
        checks.last_clean_ms[0] = None;
        checks.anchor_ms = Some(NOW_MS - 4 * DAY);
    });
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "从开始服务或第一次尝试算起已经 4 天，还没有一次干净的检查"
    );
}

#[test]
fn buckets_unread_for_32_days_after_the_first_cycle_are_listed() {
    let (state, detail) = read(|checks| {
        checks.repository_since_ms = Some(NOW_MS - 40 * DAY);
        for slot in &mut checks.last_clean_ms {
            *slot = Some(NOW_MS - DAY);
        }
        for index in [2, 4, 6, 8, 10, 12, 14, 16, 18, 20] {
            checks.last_clean_ms[index] = None;
        }
    });
    assert_eq!(state, "degraded");
    assert_eq!(
        detail,
        "首轮之后每组 32 天内都要读一遍：第 3、5、7、9、11、13、15、17 组等 10 组超过 32 天没读干净"
    );
}

#[test]
fn unknown_cases_never_read_as_normal() {
    let (state, detail) = read(|checks| checks.latest_record_ms = Some(NOW_MS + MINUTE));
    assert_eq!(state, "unknown");
    assert!(detail.starts_with("本地时钟早于账本里的检查时刻"));
    let (state, detail) = read(|checks| {
        checks.last_clean_ms[0] = None;
        checks.running = Some(RunningCheck {
            bucket: 1,
            since_ms: NOW_MS - MINUTE,
        });
    });
    assert_eq!(state, "unknown");
    assert_eq!(
        detail,
        "还没有干净的检查，离 72 小时的期限还有 2 天 22 小时；第 1 组正在查"
    );
    let (state, detail) = section(RepositoryCheckEvidence::Unreadable);
    assert_eq!(state, "unknown");
    assert_eq!(detail, "这一次没读到账本里的仓库检查事件，状态未知");
    let (state, detail) = section(RepositoryCheckEvidence::Disabled);
    assert_eq!(state, "unknown");
    assert!(detail.starts_with("备份生产者没有启用"));
}
