//! 固定消息只能表达备份作业，不能借通用 AuditEvent 携带其他动作或正文。
use serde_json::{Value, json};
use std::num::NonZeroU64;
use studiolo_audit_ledger::{AuditEvent, AuditLedger, AuditPhase, MemoryAuditLedger};
use studiolo_backup::worker::{
    audit::{AUDIT_MESSAGE_MAX_BYTES, BackupAuditRequest},
    job::backup_intent,
    recovery_point::BackupTargetKind,
    state::JobKey,
};
use studiolo_core::checkpoint::CheckpointId;
const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
fn key() -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(7).unwrap(),
        checkpoint_id: CheckpointId::parse("audit-wire").unwrap(),
    }
}
fn event(value: Value) -> AuditEvent {
    serde_json::from_value(value).unwrap()
}
fn intent() -> AuditEvent {
    backup_intent(&key(), TARGET).unwrap()
}
fn outcome(result: &str) -> AuditEvent {
    let mut value = serde_json::to_value(intent()).unwrap();
    value["phase"] = json!("outcome");
    value["payload"] = json!({"job_sequence":7,"checkpoint_id":"audit-wire","result":result,"recorded_after_checkpoint_cutoff":true});
    match result {
        "failed" => value["payload"]["reason"] = json!("upload_rejected"),
        "unverified" => value["payload"]["reason"] = json!("no_real_repository"),
        "succeeded" => {
            for (name, field) in [
                ("data_snapshot", json!("ab".repeat(32))),
                ("final_snapshot", json!("cd".repeat(32))),
                ("data_verified_at", json!(2000)),
                ("finalized_at", json!(3000)),
                ("objects_complete", json!(true)),
                ("checksum_verified", json!(true)),
                ("paired_checkpoint_reference", json!(true)),
            ] {
                value["payload"][name] = field;
            }
        }
        _ => unreachable!(),
    }
    event(value)
}
#[test]
fn original_events_and_replies_roundtrip_and_bind_the_request() {
    for event in [
        intent(),
        outcome("failed"),
        outcome("unverified"),
        outcome("succeeded"),
    ] {
        let request = BackupAuditRequest::append(key(), TARGET, event.clone()).unwrap();
        let bytes = request.encode().unwrap();
        let decoded = BackupAuditRequest::decode(&bytes).unwrap();
        assert_eq!(decoded.event(), Some(&event));
        let ledger = MemoryAuditLedger::new();
        let record = ledger.append(event.clone()).unwrap();
        let reply = decoded.encode_reply(Some(record.clone())).unwrap();
        assert_eq!(request.decode_reply(&reply).unwrap(), Some(record.clone()));
        let lookup = BackupAuditRequest::read(key(), TARGET, event.phase());
        assert!(lookup.decode_reply(&reply).is_err());
        let reply = lookup.encode_reply(Some(record.clone())).unwrap();
        assert_eq!(lookup.decode_reply(&reply).unwrap(), Some(record));
        assert!(request.encode_reply(None).is_err());
    }
    let read = BackupAuditRequest::read(key(), TARGET, AuditPhase::Outcome);
    assert!(
        read.decode_reply(&read.encode_reply(None).unwrap())
            .unwrap()
            .is_none()
    );
}
#[test]
fn foreign_actions_subjects_extra_content_and_mismatched_jobs_are_rejected() {
    let original = serde_json::to_value(intent()).unwrap();
    for (pointer, value) in [
        ("/action", json!("policy.activate")),
        ("/event_id", json!("other")),
        ("/subject/tool", json!("vault.write")),
        ("/payload/job_sequence", json!(8)),
        ("/payload/checkpoint_id", json!("another")),
        ("/payload/target_kind", json!("local_temporary_repository")),
        ("/payload/phase_note", json!("later")),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(
            BackupAuditRequest::append(key(), TARGET, event(changed)).is_err(),
            "{pointer}"
        );
    }
    let mut changed = original.clone();
    changed["payload"]["body"] = json!("private body");
    assert!(BackupAuditRequest::append(key(), TARGET, event(changed)).is_err());
    let mut changed = original;
    changed["payload"]["body"] = json!("x".repeat(AUDIT_MESSAGE_MAX_BYTES));
    assert!(BackupAuditRequest::append(key(), TARGET, event(changed)).is_err());
}
#[test]
fn success_requires_complete_distinct_snapshot_ids_and_consistent_times() {
    for (name, value) in [
        ("data_snapshot", json!("abcdef12")),
        ("final_snapshot", json!("ab".repeat(32))),
        ("data_verified_at", json!(-1)),
        ("finalized_at", json!(1000)),
        ("objects_complete", json!(false)),
        ("checksum_verified", json!(false)),
        ("paired_checkpoint_reference", json!(false)),
        ("recorded_after_checkpoint_cutoff", json!(false)),
        ("unknown_field", json!(true)),
    ] {
        let mut changed = serde_json::to_value(outcome("succeeded")).unwrap();
        changed["payload"][name] = value;
        assert!(
            BackupAuditRequest::append(key(), TARGET, event(changed)).is_err(),
            "{name}"
        );
    }
}
#[test]
fn unknown_fields_duplicate_fields_noncanonical_encoding_and_damaged_receipts_fail() {
    let request = BackupAuditRequest::append(key(), TARGET, intent()).unwrap();
    let bytes = request.encode().unwrap();
    assert!(BackupAuditRequest::decode(&[b'x'; AUDIT_MESSAGE_MAX_BYTES + 1]).is_err());
    let unknown = String::from_utf8(bytes.clone()).unwrap().replacen(
        "\"event\":{",
        "\"event\":{\"unknown\":true,",
        1,
    );
    assert!(BackupAuditRequest::decode(unknown.as_bytes()).is_err());
    let changed: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(BackupAuditRequest::decode(&serde_json::to_vec_pretty(&changed).unwrap()).is_err());
    let duplicate =
        String::from_utf8(bytes)
            .unwrap()
            .replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
    assert!(BackupAuditRequest::decode(duplicate.as_bytes()).is_err());
    let record = MemoryAuditLedger::new().append(intent()).unwrap();
    let damaged = studiolo_audit_ledger::AuditRecord::from_parts(
        0,
        record.recorded_at(),
        record.previous_hash(),
        record.record_hash(),
        record.event().clone(),
    );
    assert!(request.encode_reply(Some(damaged)).is_err());
    let mut reply: Value =
        serde_json::from_slice(&request.encode_reply(Some(record)).unwrap()).unwrap();
    reply["record"]["offset"] = json!(0);
    assert!(
        request
            .decode_reply(&serde_json::to_vec(&reply).unwrap())
            .is_err()
    );
}
