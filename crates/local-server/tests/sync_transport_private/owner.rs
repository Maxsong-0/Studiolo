//! 传输 owner 装配契约：固定审计映射、回执自检与只读投影。
//!
//! 不启动进程、不联网、不打开真实运行槽：`SyncTransportOwner` 需要一个当前拿不到的
//! `VerifiedLinuxBackend`，因此这里只覆盖装配层自己能证明的部分。

use super::*;
use studiolo_audit_ledger::{AuditLedger, AuditPhase, MemoryAuditLedger};
use studiolo_sync_transport::owned_run::{
    ActiveSummary, AuditPayload, AuditPhaseKind, ExitObservation, OutcomeSummary, Quiescence,
    RequesterSource, ResourceCounts, RunBudget, RunId, RunObservation, RunOperation, RunPhase,
    RunStatus, StopReason, TRANSPORT_RUN_ACTION, TerminalSummary,
};

fn budget() -> RunBudget {
    RunBudget {
        deadline_seconds: 3600,
        stop_grace_seconds: 30,
        stdout_bytes: 262_144,
        stderr_bytes: 262_144,
        config_json_bytes: 65_536,
        private_state_bytes: 67_108_864,
        log_bytes: 268_435_456,
        memory_bytes: 2_147_483_648,
        pids_max: 64,
        cpu_max_percent: 200,
    }
}

fn payload(outcome: Option<OutcomeSummary>) -> AuditPayload {
    AuditPayload {
        schema_version: 1,
        run_id: RunId::from_bytes([0x1A; 16]),
        sequence: 7,
        operation: RunOperation::RunOnce,
        binding_digest: [0x11; 32],
        runtime_digest: [0x22; 32],
        backend_digest: [0x33; 32],
        budget: budget(),
        requester: RequesterSource::OwnerSession,
        outcome,
    }
}

/// 收集 JSON 里的全部字符串叶子。
fn string_leaves(value: &serde_json::Value) -> Vec<String> {
    let mut pending = vec![value];
    let mut leaves = Vec::new();
    while let Some(current) = pending.pop() {
        match current {
            serde_json::Value::String(text) => leaves.push(text.clone()),
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(object) => pending.extend(object.values()),
            _ => {}
        }
    }
    leaves
}

fn outcome() -> OutcomeSummary {
    OutcomeSummary {
        container_created: true,
        stop_reason: Some(StopReason::OwnerRequested),
        exit_observed: true,
        quiescence: Quiescence::Confirmed,
        counts: ResourceCounts {
            stdout_bytes: 12,
            stderr_bytes: 0,
            output_over_budget: false,
        },
    }
}

#[test]
fn a_transport_intent_event_carries_the_fixed_action_and_only_closed_digest_fields() {
    let event = transport_event(
        "sync.transport.run/1a1a/intent",
        AuditPhaseKind::Intent,
        &payload(None),
    )
    .expect("固定候选可以映射成审计事件");

    assert_eq!(event.action(), TRANSPORT_RUN_ACTION);
    assert_eq!(event.phase(), AuditPhase::Intent);
    let value = event.payload().as_value();
    let object = value.as_object().expect("payload 是命名字段对象");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "backend_digest",
            "binding_digest",
            "budget",
            "operation",
            "outcome",
            "requester",
            "run",
            "runtime_digest",
            "schema_version",
            "sequence",
        ]
    );
    // Intent 阶段没有 Outcome 摘要。
    assert!(object["outcome"].is_null());
    // 所有字符串叶子只能是摘要或固定枚举词：没有路径、没有正文、没有凭据。
    for leaf in string_leaves(value) {
        assert!(leaf.len() <= 64, "{leaf}");
        assert!(!leaf.contains('/'), "{leaf}");
        assert!(!leaf.chars().any(char::is_whitespace), "{leaf}");
        for forbidden in ["token", "password", "secret", "cookie", "argv"] {
            assert!(!leaf.contains(forbidden), "{leaf}");
        }
    }
}

#[test]
fn a_transport_outcome_event_reports_observation_facts_and_never_claims_success() {
    let event = transport_event(
        "sync.transport.run/1a1a/outcome",
        AuditPhaseKind::Outcome,
        &payload(Some(outcome())),
    )
    .expect("固定候选可以映射成审计事件");

    assert_eq!(event.phase(), AuditPhase::Outcome);
    let value = event.payload().as_value();
    let summary = &value["outcome"];
    assert_eq!(summary["exit_observed"], serde_json::json!(true));
    assert_eq!(summary["quiescence"], serde_json::json!("confirmed"));
    assert_eq!(summary["stop_reason"], serde_json::json!("OwnerRequested"));
    assert_eq!(summary["output_over_budget"], serde_json::json!(false));
    let rendered = value.to_string();
    // 审计里既不出现「同步成功」，也不出现任何送达声明。
    for forbidden in ["synced", "delivered", "committed", "success"] {
        assert!(!rendered.contains(forbidden), "{forbidden}");
    }
    for leaf in string_leaves(value) {
        assert!(!leaf.contains('/'), "{leaf}");
        assert!(leaf.len() <= 64, "{leaf}");
    }
}

#[test]
fn an_invalid_event_identifier_never_produces_an_audit_event() {
    assert_eq!(
        transport_event("", AuditPhaseKind::Intent, &payload(None)).err(),
        Some(TransportAuditError::Mismatch)
    );
    assert_eq!(
        transport_event("bad\nid", AuditPhaseKind::Intent, &payload(None)).err(),
        Some(TransportAuditError::Mismatch)
    );
}

#[test]
fn one_flush_never_exceeds_the_two_pending_events_an_active_slot_can_hold() {
    assert_eq!(flush_budget(0).err(), Some(TransportAuditError::Budget));
    assert_eq!(flush_budget(3).err(), Some(TransportAuditError::Budget));
    assert_eq!(flush_budget(1), Ok(()));
    assert_eq!(flush_budget(2), Ok(()));
}

#[test]
fn a_receipt_is_only_accepted_when_it_carries_exactly_the_event_that_was_appended() {
    let ledger = MemoryAuditLedger::new();
    let intent = transport_event(
        "sync.transport.run/1a1a/intent",
        AuditPhaseKind::Intent,
        &payload(None),
    )
    .expect("固定候选可以映射成审计事件");
    let record = ledger.append(intent.clone()).expect("可以追加");

    let (offset, hash) = verify_receipt(&intent, &record).expect("自洽回执可以接受");
    assert_eq!(offset, record.offset());
    assert_ne!(hash, [0u8; 32]);
    assert_eq!(hash.len(), 32);

    // 回执必须绑定同一条事件：拿另一条事件的回执一律拒绝。
    let other = transport_event(
        "sync.transport.run/1a1a/outcome",
        AuditPhaseKind::Outcome,
        &payload(Some(outcome())),
    )
    .expect("固定候选可以映射成审计事件");
    assert_eq!(
        verify_receipt(&other, &record).err(),
        Some(TransportAuditError::Mismatch)
    );
}

#[test]
fn the_status_projection_keeps_phase_quiescence_and_exit_as_separate_facts() {
    let observation = RunObservation {
        phase: RunPhase::StopRequested,
        quiescence: Quiescence::Unconfirmed,
        exit: ExitObservation::Signal(15),
        stop_reason: Some(StopReason::DeadlineExceeded),
    };
    let status = RunStatus {
        generation: 9,
        next_sequence: 4,
        audit_pending: true,
        active: Some(ActiveSummary {
            run_id: RunId::from_bytes([0x2B; 16]),
            sequence: 3,
            operation: RunOperation::RunOnce,
            observation,
        }),
        last_terminal: Some(TerminalSummary {
            run_id: RunId::from_bytes([0x3C; 16]),
            sequence: 2,
            operation: RunOperation::InspectConfig,
            observation: RunObservation {
                phase: RunPhase::Closed,
                quiescence: Quiescence::Confirmed,
                exit: ExitObservation::NotObserved,
                stop_reason: None,
            },
            counts: ResourceCounts::default(),
        }),
    };

    let view = status_view(&status);
    let active = view.active.expect("活动运行视图");
    assert_eq!(active.phase, "StopRequested");
    assert!(!active.quiescence_confirmed);
    assert!(active.exit_observed);
    assert_eq!(active.exit_signal, Some(15));
    assert_eq!(active.exit_code, None);
    assert_eq!(active.stop_reason.as_deref(), Some("DeadlineExceeded"));

    let terminal = view.last_terminal.expect("闭合摘要视图");
    // 静止已确认但退出结果未知：两件事分开报告，不互相替代。
    assert!(terminal.quiescence_confirmed);
    assert!(!terminal.exit_observed);
    assert!(view.audit_pending);
}

#[test]
fn an_approval_document_that_is_not_explicitly_enabled_never_assembles_a_capability() {
    // 这里不建真实私有根：默认关闭这一条在解析阶段就必须拦下。
    let disabled = br#"{"schema_version":1,"enabled":false}"#;
    assert!(studiolo_host_config::sync_transport::SyncTransportApproval::parse(disabled).is_err());
    assert!(studiolo_host_config::sync_transport::SyncTransportApproval::parse(b"{}").is_err());
}
