use super::{build, canonical};
use crate::{AuditEvent, AuditEventId, AuditPhase, IntegrityHash, SanitizedPayload};
use serde_json::json;

#[test]
fn v1_encoding_and_hash_are_fixed_by_an_independent_vector() {
    let event = AuditEvent::new(
        AuditEventId::parse("vector-1").unwrap(),
        AuditPhase::Intent,
        "audit.test",
        SanitizedPayload::try_from(json!({"z": [true, null, {"二": 2, "a": 1}], "a": "审计\n"}))
            .unwrap(),
    );
    let timestamp = jiff::Timestamp::from_second(0).unwrap();
    let encoded = canonical(&(1_u64, timestamp, IntegrityHash::genesis(), &event)).unwrap();
    assert_eq!(
        encoded,
        concat!(
            "[1,\"1970-01-01T00:00:00Z\",[164,11,244,160,196,44,36,26,74,2,125,29,162,198,52,180,139,6,45,243,144,40,140,210,77,10,158,141,140,60,90,216],",
            "{\"action\":\"audit.test\",\"event_id\":\"vector-1\",\"payload\":{\"a\":\"审计\\n\",\"z\":[true,null,{\"a\":1,\"二\":2}]},",
            "\"phase\":\"intent\",\"subject\":{\"attempt_id\":null,\"task_id\":null,\"tool\":null}}]"
        )
    );
    let record = build(1, timestamp, IntegrityHash::genesis(), event).unwrap();
    assert_eq!(
        record.record_hash().to_hex(),
        "4d1b38103973dd2df65baf9398e5ae4ac674fd69b96c56fccfa0bf496dfe8b3e"
    );
}

#[test]
fn recursive_object_order_is_canonical_but_array_order_is_preserved() {
    let first: serde_json::Value =
        serde_json::from_str(r#"{"z":{"b":2,"a":1},"a":[2,1]}"#).unwrap();
    let second = json!({"a": [2,1], "z": {"a": 1, "b": 2}});
    assert_eq!(canonical(&first).unwrap(), canonical(&second).unwrap());
    assert_ne!(
        canonical(&first).unwrap(),
        canonical(&json!({"a": [1,2], "z": {"a": 1,"b": 2}})).unwrap()
    );
}
