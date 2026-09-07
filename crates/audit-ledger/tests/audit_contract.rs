//! Public contract tests for the in-memory AuditLedger adapter.

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditSubject,
    IntegrityHash, MemoryAuditLedger, SanitizedPayload,
};
use studiolo_core::{AttemptId, TaskId, ToolId};

fn payload(value: serde_json::Value) -> SanitizedPayload {
    SanitizedPayload::try_from(value).expect("test payload is sanitized")
}

fn event(id: &str, phase: AuditPhase, action: &str, value: serde_json::Value) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(id).expect("valid event id"),
        phase,
        action,
        payload(value),
    )
}

#[test]
fn audit_ledger_is_object_safe_and_duplicate_append_is_idempotent() {
    let ledger = MemoryAuditLedger::new();
    let seam: &dyn AuditLedger = &ledger;
    let original = event(
        "call-001",
        AuditPhase::Intent,
        "gateway.tool_call",
        json!({"tool": "web.fetch"}),
    );

    let first = seam.append(original.clone()).unwrap();
    let duplicate = seam.append(original).unwrap();

    assert_eq!(duplicate, first);
    assert_eq!(seam.query(&AuditFilter::default()).unwrap(), vec![first]);
}

#[test]
fn same_event_and_phase_cannot_be_reused_for_different_content() {
    let ledger = MemoryAuditLedger::new();
    ledger
        .append(event(
            "call-002",
            AuditPhase::Outcome,
            "gateway.tool_call",
            json!({"decision": "allow"}),
        ))
        .unwrap();

    let error = ledger
        .append(event(
            "call-002",
            AuditPhase::Outcome,
            "gateway.tool_call",
            json!({"decision": "deny"}),
        ))
        .unwrap_err();

    assert_eq!(
        error,
        AuditError::IdempotencyConflict {
            event_id: AuditEventId::parse("call-002").unwrap(),
            phase: AuditPhase::Outcome,
        }
    );
}

#[test]
fn records_are_linked_by_sha256_hashes() {
    assert_eq!(
        IntegrityHash::sha256(b"abc").to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    let ledger = MemoryAuditLedger::new();
    let first = ledger
        .append(event(
            "call-003",
            AuditPhase::Intent,
            "gateway.tool_call",
            json!({"tool": "db.query"}),
        ))
        .unwrap();
    let second = ledger
        .append(event(
            "call-003",
            AuditPhase::Outcome,
            "gateway.tool_call",
            json!({"decision": "allow"}),
        ))
        .unwrap();

    assert_eq!(first.previous_hash(), IntegrityHash::genesis());
    assert_eq!(second.previous_hash(), first.record_hash());
    assert_ne!(second.record_hash(), first.record_hash());
}

#[test]
fn payload_rejects_secret_fields_at_any_depth() {
    for (value, expected_path) in [
        (json!({"token": "value"}), "token"),
        (
            json!({"request": {"headers": {"Authorization": "value"}}}),
            "request.headers.Authorization",
        ),
        (json!({"items": [{"api-key": "value"}]}), "items[0].api-key"),
        (json!({"api_key": "value"}), "api_key"),
        (json!({"secret": "value"}), "secret"),
        (json!({"client_secret": "value"}), "client_secret"),
        (json!({"password": "value"}), "password"),
        (json!({"cookie": "value"}), "cookie"),
        (json!({"apiKey": "value"}), "apiKey"),
        (json!({"x-api-key": "value"}), "x-api-key"),
        (json!({"PRIVATE_KEY": "value"}), "PRIVATE_KEY"),
        (json!({"refreshToken": "value"}), "refreshToken"),
        (json!({"passwd": "value"}), "passwd"),
        (json!({"user": {"pwd": "value"}}), "user.pwd"),
        (json!({"recovery_code": "value"}), "recovery_code"),
        (json!({"totp_seed": "value"}), "totp_seed"),
        (json!({"bearer": "value"}), "bearer"),
        (json!({"credentials": {}}), "credentials"),
        // 复数形态与「凭据本体」后缀（2026-09-06 T-032）。
        (json!({"tokens": ["a"]}), "tokens"),
        (json!({"api_keys": {"cf": "v"}}), "api_keys"),
        (json!({"secrets": {}}), "secrets"),
        (json!({"passwords": []}), "passwords"),
        (json!({"cookies": "a=b"}), "cookies"),
        (json!({"access_keys": []}), "access_keys"),
        (json!({"seed_hex": "00"}), "seed_hex"),
        (json!({"token_value": "v"}), "token_value"),
        (json!({"secret_material": "v"}), "secret_material"),
        (json!({"api_key_b64": "v"}), "api_key_b64"),
        (json!({"passwordPlaintext": "v"}), "passwordPlaintext"),
    ] {
        assert_eq!(
            SanitizedPayload::try_from(value).unwrap_err(),
            AuditError::SecretField {
                path: expected_path.to_owned(),
            }
        );
    }

    // 计数、标识与单独的 key/code 不是凭据，不能因为词根相似被拒绝。
    for value in [
        json!({"token_count": 12}),
        json!({"tokens_used": 12}),
        json!({"session_id": "s-1"}),
        json!({"key": "orders"}),
        json!({"keys": ["a"]}),
        json!({"kid": "k1"}),
        json!({"status_code": 200}),
        json!({"seed_ref": "slot-a"}),
        json!({"recovery_candidate_available": true}),
        json!({"tokenizer": "cl100k"}),
        json!({"token_digest": "ab"}),
        json!({"status": "pass"}),
        json!({"addresses": ["a"]}),
        json!({"value": 1}),
        json!({"hex": "ff"}),
    ] {
        assert!(SanitizedPayload::try_from(value).is_ok());
    }
    assert!(
        SanitizedPayload::try_from(json!({
            "task": "T-1",
            "decision": "deny",
            "reason": "tool_not_allowed"
        }))
        .is_ok()
    );
}

#[test]
fn deserialization_cannot_bypass_payload_or_event_id_validation() {
    let payload_error = serde_json::from_value::<SanitizedPayload>(json!({
        "request": {"authorization": "value"}
    }))
    .unwrap_err();
    assert!(payload_error.to_string().contains("request.authorization"));

    assert!(serde_json::from_str::<AuditEventId>("\"\"").is_err());
    assert!(serde_json::from_str::<AuditEventId>("\"bad\\nidentifier\"").is_err());
}

#[test]
fn query_filters_by_subject_phase_action_and_offset() {
    let ledger = MemoryAuditLedger::new();
    let task = TaskId::new();
    let other_task = TaskId::new();
    let attempt = AttemptId::new();
    let tool = ToolId::from_static("web.fetch");

    let matching = event(
        "call-004",
        AuditPhase::Intent,
        "gateway.tool_call",
        json!({"decision": "pending"}),
    )
    .with_subject(
        AuditSubject::default()
            .with_task(task)
            .with_attempt(attempt)
            .with_tool(tool),
    );
    let first = ledger.append(matching).unwrap();
    ledger
        .append(
            event(
                "call-004",
                AuditPhase::Outcome,
                "gateway.tool_call",
                json!({"decision": "allow"}),
            )
            .with_subject(AuditSubject::default().with_task(task)),
        )
        .unwrap();
    ledger
        .append(
            event(
                "task-001",
                AuditPhase::Intent,
                "task.command",
                json!({"command": "run"}),
            )
            .with_subject(AuditSubject::default().with_task(other_task)),
        )
        .unwrap();

    let records = ledger
        .query(
            &AuditFilter::default()
                .with_task(task)
                .with_attempt(attempt)
                .with_tool(ToolId::from_static("web.fetch"))
                .with_phase(AuditPhase::Intent)
                .with_action("gateway.tool_call")
                .after_offset(0)
                .with_limit(1),
        )
        .unwrap();

    assert_eq!(records, vec![first]);
}

#[test]
fn filter_and_record_parts_are_available_to_adapter_implementors() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let tool = ToolId::from_static("web.fetch");
    let event_id = AuditEventId::parse("adapter-event").unwrap();
    let filter = AuditFilter::default()
        .with_event_id(event_id.clone())
        .with_task(task)
        .with_attempt(attempt)
        .with_tool(tool.clone())
        .with_phase(AuditPhase::Outcome)
        .with_action("gateway.tool_call")
        .after_offset(9)
        .with_limit(20);

    assert_eq!(filter.event_id(), Some(&event_id));
    assert_eq!(filter.task_id(), Some(task));
    assert_eq!(filter.attempt_id(), Some(attempt));
    assert_eq!(filter.tool(), Some(&tool));
    assert_eq!(filter.phase(), Some(AuditPhase::Outcome));
    assert_eq!(filter.action(), Some("gateway.tool_call"));
    assert_eq!(filter.offset_exclusive(), Some(9));
    assert_eq!(filter.limit(), Some(20));

    let event = event(
        "adapter-event",
        AuditPhase::Outcome,
        "gateway.tool_call",
        json!({"decision": "allow"}),
    );
    let record = studiolo_audit_ledger::AuditRecord::from_parts(
        10,
        jiff::Timestamp::now(),
        IntegrityHash::genesis(),
        IntegrityHash::sha256(b"adapter record"),
        event.clone(),
    );
    assert_eq!(record.event(), &event);
}
