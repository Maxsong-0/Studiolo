//! T012 v1规范记录的字段策略升级：旧链自洽不绕过新词法检查，不自动重写旧记录。

#![cfg(unix)]

mod support;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    SanitizedPayload, SqliteAuditLedger,
};

fn event(id: &str) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(id).unwrap(),
        AuditPhase::Intent,
        "legacy.field-policy",
        SanitizedPayload::try_from(json!({"decision": "allow"})).unwrap(),
    )
}

// 独立组装已经冻结的T012 v1编码；正对照还会通过实际Adapter验链，不能
// 只写入坏hash再把失败误算成词法策略生效。所有值均为无秘密的虚构夹具。
fn legacy_record(record: &AuditRecord, key: &str) -> (String, String) {
    let mut value = serde_json::to_value(record).unwrap();
    value["event"]["payload"] = json!({key: "fictional-legacy-value"});
    let frame = json!([
        value["offset"],
        value["recorded_at"],
        value["previous_hash"],
        value["event"]
    ]);
    let mut input = b"studiolo-audit-record-v1\0".to_vec();
    for byte in value["previous_hash"].as_array().unwrap() {
        input.push(u8::try_from(byte.as_u64().unwrap()).unwrap());
    }
    input.extend(serde_json::to_vec(&frame).unwrap());
    let digest = Sha256::digest(input);
    let head = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    value["record_hash"] = json!(digest.to_vec());
    (serde_json::to_string(&value).unwrap(), head)
}

#[test]
fn t012_records_apply_the_new_field_policy_without_rewriting_history() {
    // seed_ref和apiKey在升级前都允许；后者在T027词法策略下明确拒绝。
    for key in ["seed_ref", "apiKey"] {
        let temp = support::private_dir();
        let root = temp.path().canonicalize().unwrap();
        let ledger = SqliteAuditLedger::open(&root).unwrap();
        let receipt = ledger.append(event("legacy")).unwrap();
        let (encoded, head) = legacy_record(&receipt, key);
        let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        connection
            .execute("UPDATE audit_events SET record_json=?1", [&encoded])
            .unwrap();
        connection
            .execute("UPDATE audit_meta SET last_hash=?1", [&head])
            .unwrap();
        connection.execute_batch("COMMIT").unwrap();
        drop(connection);

        if key == "seed_ref" {
            let expected: AuditRecord = serde_json::from_str(&encoded).unwrap();
            expected.verify_integrity().unwrap();
            assert_eq!(
                ledger.query(&AuditFilter::default()).unwrap(),
                vec![expected.clone()]
            );
            assert_eq!(ledger.append(expected.event().clone()).unwrap(), expected);
            drop(ledger);
            let reopened = SqliteAuditLedger::open_existing(&root).unwrap();
            assert_eq!(
                reopened.query(&AuditFilter::default()).unwrap(),
                vec![expected.clone()]
            );
            assert_eq!(reopened.append(expected.event().clone()).unwrap(), expected);
        } else {
            assert!(serde_json::from_str::<AuditRecord>(&encoded).is_err());
            assert_eq!(
                ledger
                    .query(&AuditFilter::default().with_limit(0))
                    .unwrap_err(),
                AuditError::CorruptStore
            );
            assert_eq!(
                ledger.append(event("unrelated-new-event")).unwrap_err(),
                AuditError::CorruptStore
            );
            drop(ledger);
            assert!(matches!(
                SqliteAuditLedger::open_existing(&root),
                Err(AuditError::CorruptStore)
            ));
        }

        let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
        let stored: (String, i64, String, i64) = connection.query_row(
            "SELECT record_json,last_offset,last_hash,(SELECT COUNT(*) FROM audit_events) FROM audit_events,audit_meta",
            [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(stored, (encoded, 1, head, 1), "rewrote {key} legacy record");
    }
}

#[test]
fn deserialization_rejects_new_lexical_secrets_inside_arrays() {
    for key in ["apiKey", "PRIVATE_KEY", "recovery_code", "totp_seed"] {
        let value: Value = json!({"items": [{key: "fictional"}]});
        assert!(
            serde_json::from_value::<SanitizedPayload>(value).is_err(),
            "accepted {key}"
        );
    }
}

#[test]
fn lexical_rejection_safely_discards_an_unvisited_deep_value() {
    let mut nested = Value::Null;
    for _ in 0..10_000 {
        nested = Value::Array(vec![nested]);
    }
    let mut object = serde_json::Map::new();
    object.insert("apiKey".into(), nested);
    assert_eq!(
        SanitizedPayload::try_from(Value::Object(object)).unwrap_err(),
        AuditError::SecretField {
            path: "apiKey".into()
        },
    );
}
