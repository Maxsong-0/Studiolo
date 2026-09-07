//! 审计编码深度、浮点精确回放与失败不推进契约。

#![cfg(unix)]

mod support;

use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger, SanitizedPayload, SqliteAuditLedger,
};

fn event(id: &str, value: Value) -> Result<AuditEvent, AuditError> {
    Ok(AuditEvent::new(
        AuditEventId::parse(id)?,
        AuditPhase::Intent,
        "test.encoding",
        SanitizedPayload::try_from(value)?,
    ))
}

fn append_nested(
    ledger: &dyn AuditLedger,
    id: &str,
    depth: usize,
) -> Result<AuditRecord, AuditError> {
    let mut value = Value::Null;
    for _ in 0..depth {
        value = Value::Array(vec![value]);
    }
    ledger.append(event(id, value)?)
}

fn assert_depth_contract(ledger: &dyn AuditLedger) {
    assert_eq!(append_nested(ledger, "boundary", 64).unwrap().offset(), 1);
    for depth in [65, 130, 10_000] {
        assert_eq!(
            append_nested(ledger, "too-deep", depth).unwrap_err(),
            AuditError::PayloadTooDeep
        );
    }
    assert_eq!(
        append_nested(ledger, "after-rejection", 0)
            .unwrap()
            .offset(),
        2
    );
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}

#[test]
fn nesting_boundary_is_shared_and_rejected_values_do_not_advance_offsets() {
    assert_depth_contract(&MemoryAuditLedger::new());
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    assert_depth_contract(&ledger);
    let records = ledger.query(&AuditFilter::default()).unwrap();
    drop(ledger);
    assert_eq!(
        SqliteAuditLedger::open(&root)
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap(),
        records
    );
}

#[test]
fn lossy_float_round_trip_is_rejected_before_sqlite_commit() {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let lossy = json!({"coordinate": 51.248178375505404_f64});
    // 当前已锁依赖未启用 float_roundtrip；此值能重现默认解码的精度变化。
    let decoded: Value = serde_json::from_str(&serde_json::to_string(&lossy).unwrap()).unwrap();
    assert_ne!(decoded, lossy);
    assert_eq!(
        ledger.append(event("float", lossy).unwrap()).unwrap_err(),
        AuditError::Serialization
    );
    assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    let exact = event("float", json!({"coordinate": 0.5})).unwrap();
    let record = ledger.append(exact.clone()).unwrap();
    assert_eq!(record.offset(), 1);
    assert_eq!(ledger.append(exact.clone()).unwrap(), record);
    drop(ledger);
    let reopened = SqliteAuditLedger::open(&root).unwrap();
    assert_eq!(reopened.append(exact).unwrap(), record);
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![record]
    );
}
