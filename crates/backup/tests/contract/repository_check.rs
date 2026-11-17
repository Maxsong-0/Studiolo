//! 仓库抽查的共用事实：组号、结果组合、仓库摘要、审计事件写法和 restic 输出归类。

use std::num::NonZeroU64;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_backup::worker::{
    recovery_point::BackupTargetKind,
    repository_check::{
        CHECK_BUCKETS, CheckAssignment, CheckBucket, CheckEvent, CheckKey, CheckOutcome,
        CheckPurpose, CheckRecorder, CheckResult, IncompleteReason, REPOSITORY_CHECK_ACTION,
        RepositoryDigest, check_abandoned, check_intent, check_outcome, classify_check_output,
        read_check_event,
    },
    restic::RepositoryRef,
};

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;

fn key(sequence: u64, bucket: u8) -> CheckKey {
    CheckKey {
        sequence: NonZeroU64::new(sequence).unwrap(),
        bucket: CheckBucket::new(bucket).unwrap(),
    }
}

fn digest() -> RepositoryDigest {
    RepositoryDigest::of(&RepositoryRef::parse("rest:https://backup.example.test/repo/").unwrap())
}

#[test]
fn buckets_rotate_through_thirty_groups_and_name_restic_subsets() {
    assert_eq!(CHECK_BUCKETS, 30);
    let all: Vec<u8> = CheckBucket::all().map(CheckBucket::get).collect();
    assert_eq!(all, (1..=30).collect::<Vec<u8>>());
    assert_eq!(CheckBucket::FIRST.get(), 1);
    assert_eq!(CheckBucket::new(29).unwrap().following().get(), 30);
    assert_eq!(
        CheckBucket::new(30).unwrap().following(),
        CheckBucket::FIRST
    );
    assert_eq!(CheckBucket::new(9).unwrap().subset(), "9/30");
}

#[test]
fn outcomes_only_take_consistent_field_combinations() {
    assert!(CheckOutcome::problem(0, 0).is_none());
    assert_eq!(CheckOutcome::problem(0, 2).unwrap().broken_packs(), 2);
    let incomplete = CheckOutcome::incomplete(IncompleteReason::Preempted);
    assert_eq!(incomplete.result(), CheckResult::Incomplete);
    assert_eq!(incomplete.reason(), Some(IncompleteReason::Preempted));
    for outcome in [
        CheckOutcome::clean(),
        CheckOutcome::problem(3, 1).unwrap(),
        incomplete,
    ] {
        let text = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            serde_json::from_str::<CheckOutcome>(&text).unwrap(),
            outcome
        );
    }
    for broken in [
        r#"{"result":"clean","reason":null,"errors":1,"broken_packs":0}"#,
        r#"{"result":"clean","reason":"deadline","errors":0,"broken_packs":0}"#,
        r#"{"result":"problem","reason":null,"errors":0,"broken_packs":0}"#,
        r#"{"result":"problem","reason":"protocol","errors":1,"broken_packs":0}"#,
        r#"{"result":"incomplete","reason":null,"errors":0,"broken_packs":0}"#,
        r#"{"result":"incomplete","reason":"deadline","errors":2,"broken_packs":0}"#,
        r#"{"result":"clean","reason":null,"errors":0,"broken_packs":0,"extra":1}"#,
        r#"{"result":"clean","errors":0,"broken_packs":0}"#,
        r#"{"result":"damaged","reason":null,"errors":1,"broken_packs":1}"#,
    ] {
        assert!(
            serde_json::from_str::<CheckOutcome>(broken).is_err(),
            "{broken}"
        );
    }
}

#[test]
fn repository_digest_is_a_stable_hex_label_that_changes_with_the_repository() {
    let first = digest();
    assert_eq!(first, digest());
    assert_eq!(first.as_str().len(), 64);
    assert!(!first.as_str().contains("example"));
    let other = RepositoryDigest::of(
        &RepositoryRef::parse("rest:https://other.example.test/repo/").unwrap(),
    );
    assert_ne!(first, other);
    assert_eq!(RepositoryDigest::parse(first.as_str()), Some(first.clone()));
    for bad in [
        "",
        "abc",
        &first.as_str().to_uppercase(),
        &format!("{}0", first.as_str()),
    ] {
        assert!(RepositoryDigest::parse(bad).is_none(), "{bad}");
    }
    assert!(serde_json::from_str::<RepositoryDigest>("\"zz\"").is_err());
}

#[test]
fn intents_and_outcomes_have_one_fixed_shape_and_read_back_exactly() {
    let assignment = CheckAssignment {
        key: key(7, 12),
        purpose: CheckPurpose::Rotation,
    };
    let intent = check_intent(&assignment, TARGET).unwrap();
    assert_eq!(intent.action(), REPOSITORY_CHECK_ACTION);
    assert_eq!(intent.event_id().as_str(), "backup.repository_check:7");
    assert_eq!(intent.phase(), AuditPhase::Intent);
    assert_eq!(
        intent.payload().as_value(),
        &serde_json::json!({
            "check_sequence": 7,
            "bucket": 12,
            "buckets": 30,
            "purpose": "rotation",
            "target_kind": "dedicated_append_only_target",
            "phase_note": "intent_persisted_before_check",
        })
    );
    assert_eq!(
        read_check_event(&intent),
        Some(CheckEvent::Intent {
            assignment,
            target: TARGET
        })
    );

    let problem = CheckOutcome::problem(4, 1).unwrap();
    let reported = check_outcome(&key(7, 12), &problem, &CheckRecorder::Worker(digest())).unwrap();
    assert_eq!(reported.event_id(), intent.event_id());
    assert_eq!(reported.phase(), AuditPhase::Outcome);
    assert_eq!(reported.payload().as_value()["recorded_by"], "worker");
    assert_eq!(
        reported.payload().as_value()["repository"],
        digest().as_str()
    );
    assert_eq!(
        read_check_event(&reported),
        Some(CheckEvent::Outcome {
            key: key(7, 12),
            outcome: problem,
            recorder: CheckRecorder::Worker(digest()),
        })
    );

    let abandoned = check_abandoned(&key(8, 13)).unwrap();
    assert_eq!(abandoned.payload().as_value()["recorded_by"], "producer");
    assert_eq!(
        abandoned.payload().as_value()["repository"],
        serde_json::Value::Null
    );
    assert_eq!(
        read_check_event(&abandoned),
        Some(CheckEvent::Outcome {
            key: key(8, 13),
            outcome: CheckOutcome::incomplete(IncompleteReason::Abandoned),
            recorder: CheckRecorder::Producer,
        })
    );
}

#[test]
fn worker_and_producer_each_write_only_their_own_outcomes() {
    // worker 不能自称「中途没了」；生产者只能补「中途没了」，不能替 worker 写干净或查出问题。
    assert!(
        check_outcome(
            &key(1, 1),
            &CheckOutcome::incomplete(IncompleteReason::Abandoned),
            &CheckRecorder::Worker(digest())
        )
        .is_err()
    );
    for outcome in [
        CheckOutcome::clean(),
        CheckOutcome::problem(1, 0).unwrap(),
        CheckOutcome::incomplete(IncompleteReason::Deadline),
    ] {
        assert!(check_outcome(&key(1, 1), &outcome, &CheckRecorder::Producer).is_err());
        assert!(check_outcome(&key(1, 1), &outcome, &CheckRecorder::Worker(digest())).is_ok());
    }
}

#[test]
fn tampered_or_foreign_events_do_not_read_back() {
    let assignment = CheckAssignment {
        key: key(3, 4),
        purpose: CheckPurpose::Retry,
    };
    let intent = check_intent(&assignment, TARGET).unwrap();
    let rebuilt = |id: &str, phase, action: &str, payload: serde_json::Value| {
        AuditEvent::new(
            AuditEventId::parse(id).unwrap(),
            phase,
            action,
            SanitizedPayload::try_from(payload).unwrap(),
        )
    };
    let mut extra = intent.payload().as_value().clone();
    extra["note"] = serde_json::json!("x");
    let mut other_count = intent.payload().as_value().clone();
    other_count["buckets"] = serde_json::json!(31);
    let mut bad_bucket = intent.payload().as_value().clone();
    bad_bucket["bucket"] = serde_json::json!(0);
    for event in [
        rebuilt(
            "backup.repository_check:3",
            AuditPhase::Intent,
            REPOSITORY_CHECK_ACTION,
            extra,
        ),
        rebuilt(
            "backup.repository_check:3",
            AuditPhase::Intent,
            REPOSITORY_CHECK_ACTION,
            other_count,
        ),
        rebuilt(
            "backup.repository_check:3",
            AuditPhase::Intent,
            REPOSITORY_CHECK_ACTION,
            bad_bucket,
        ),
        // 事件键和载荷里的序号对不上：整条重新生成后比较，不只看字段。
        rebuilt(
            "backup.repository_check:9",
            AuditPhase::Intent,
            REPOSITORY_CHECK_ACTION,
            intent.payload().as_value().clone(),
        ),
        rebuilt(
            "backup.repository_check:3",
            AuditPhase::Intent,
            "backup.snapshot",
            intent.payload().as_value().clone(),
        ),
        rebuilt(
            "backup.repository_check:3",
            AuditPhase::Outcome,
            REPOSITORY_CHECK_ACTION,
            intent.payload().as_value().clone(),
        ),
    ] {
        assert_eq!(read_check_event(&event), None, "{event:?}");
    }
    let reported = check_outcome(
        &key(3, 4),
        &CheckOutcome::clean(),
        &CheckRecorder::Worker(digest()),
    )
    .unwrap();
    let mut dirty = reported.payload().as_value().clone();
    dirty["errors"] = serde_json::json!(2);
    let mut no_repository = reported.payload().as_value().clone();
    no_repository["repository"] = serde_json::Value::Null;
    for payload in [dirty, no_repository] {
        let event = rebuilt(
            "backup.repository_check:3",
            AuditPhase::Outcome,
            REPOSITORY_CHECK_ACTION,
            payload,
        );
        assert_eq!(read_check_event(&event), None, "{event:?}");
    }
}

const CLEAN: &str = r#"{"message_type":"summary","num_errors":0,"broken_packs":null,"suggest_repair_index":false,"suggest_prune":false}"#;

#[test]
fn only_a_zero_exit_with_one_error_free_summary_is_clean() {
    assert_eq!(
        classify_check_output(true, format!("{CLEAN}\n").as_bytes()),
        CheckOutcome::clean()
    );
    let empty_list = CLEAN.replace("null", "[]");
    assert_eq!(
        classify_check_output(true, empty_list.as_bytes()),
        CheckOutcome::clean()
    );
    // 别的 JSON 对象不影响归类。
    let with_other = format!("{{\"message_type\":\"status\",\"percent_done\":1}}\n{CLEAN}\n");
    assert_eq!(
        classify_check_output(true, with_other.as_bytes()),
        CheckOutcome::clean()
    );
}

#[test]
fn any_reported_error_or_broken_pack_is_a_problem_whatever_the_exit_code() {
    let pack = "a".repeat(64);
    let damaged = format!(
        r#"{{"message_type":"summary","num_errors":2,"broken_packs":["{pack}"],"suggest_repair_index":false,"suggest_prune":false}}"#
    );
    assert_eq!(
        classify_check_output(false, damaged.as_bytes()),
        CheckOutcome::problem(2, 1).unwrap()
    );
    // 连不上、凭据不对时 restic 也打 summary，num_errors 至少 1：照样是「查出问题」，不说损坏。
    let failed = CLEAN.replace("\"num_errors\":0", "\"num_errors\":1");
    assert_eq!(
        classify_check_output(false, failed.as_bytes()),
        CheckOutcome::problem(1, 0).unwrap()
    );
    assert_eq!(
        classify_check_output(true, failed.as_bytes()),
        CheckOutcome::problem(1, 0).unwrap()
    );
}

#[test]
fn missing_duplicated_or_malformed_summaries_are_never_a_verdict() {
    let protocol = CheckOutcome::incomplete(IncompleteReason::Protocol);
    let two = format!("{CLEAN}\n{CLEAN}\n");
    let cases = [
        String::new(),
        "\n".to_owned(),
        "not json\n".to_owned(),
        format!("{CLEAN}\nnot json"),
        "[1,2]\n".to_owned(),
        two,
        CLEAN.replace("\"num_errors\":0", "\"num_errors\":\"0\""),
        CLEAN.replace("\"num_errors\":0", "\"num_errors\":-1"),
        CLEAN.replace("\"num_errors\":0,", ""),
        CLEAN.replace("\"broken_packs\":null", "\"broken_packs\":[1]"),
        CLEAN.replace("\"broken_packs\":null", "\"broken_packs\":\"x\""),
        CLEAN.replace("\"broken_packs\":null,", ""),
    ];
    for case in &cases {
        assert_eq!(
            classify_check_output(true, case.as_bytes()),
            protocol,
            "{case}"
        );
    }
    // 退出码不是 0 却没报任何错误：和固定版本对不上，也不当结论。
    assert_eq!(classify_check_output(false, CLEAN.as_bytes()), protocol);
}
