//! 请求身份与完成队列回归；不使用耗时阈值判断算法复杂度。

use super::*;
use crate::{ModelRequest, ToolCall};
use serde_json::{Value, json};
use std::sync::Arc;
use studiolo_core::ToolId;

fn tool(id: &str, name: &'static str, arguments: Value) -> ToolCall {
    ToolCall::new(
        CallId::parse(id).unwrap(),
        ToolId::from_static(name),
        arguments,
    )
}

fn model(id: &str, provider: &str, name: &str, prompt: &str) -> ModelRequest {
    ModelRequest::new(CallId::parse(id).unwrap(), provider, name, prompt)
}

fn occupy<Response: Clone>(
    table: &CallTable<Response>,
    key: CallKey,
    identity: RequestIdentity,
) -> SlotGuard<'_, Response> {
    match table.admit(key, identity).unwrap() {
        CallAdmission::Proceed(guard) => guard,
        _ => panic!("fresh call did not acquire its slot"),
    }
}

#[test]
fn completed_and_evicted_slots_keep_only_fixed_request_identity() {
    assert_eq!(std::mem::size_of::<RequestIdentity>(), 32);
    let table = CallTable::new();
    let attempt = AttemptId::new();
    let output = Arc::new(());
    let released = Arc::downgrade(&output);
    let mut first = None;
    for index in 0..=MAX_RETAINED_RESULTS {
        let id = format!("body-{index}");
        let key = (attempt, CallId::parse(&id).unwrap());
        // 完整请求只活到本次调用结束，槽的类型没有持有它的字段。
        let identity = if index % 2 == 0 {
            RequestIdentity::tool(&tool(&id, "web.fetch", json!({"body": "x".repeat(4096)})))
        } else {
            RequestIdentity::model(&model(&id, "local", "fixture", &"x".repeat(4096)))
        };
        if index == 0 {
            first = Some((key.clone(), identity));
        }
        let response = if index == 0 {
            output.clone()
        } else {
            Arc::new(())
        };
        occupy(&table, key, identity).complete(response).unwrap();
        assert!(table.state.lock().unwrap().completed.len() <= MAX_RETAINED_RESULTS);
    }
    drop(output);
    assert!(released.upgrade().is_none(), "evicted response is released");
    let state = table.state.lock().unwrap();
    assert_eq!(state.slots.len(), MAX_RETAINED_RESULTS + 1);
    assert_eq!(state.completed.len(), MAX_RETAINED_RESULTS);
    assert_eq!(
        state
            .slots
            .values()
            .filter(|slot| matches!(slot, CallSlot::Done { .. }))
            .count(),
        MAX_RETAINED_RESULTS
    );
    for slot in state.slots.values() {
        let identity = match slot {
            CallSlot::Done { identity, .. } | CallSlot::Evicted(identity) => identity,
            CallSlot::InFlight => panic!("completed call left an in-flight slot"),
        };
        let _: RequestIdentity = *identity;
        assert_eq!(std::mem::size_of_val(identity), 32);
    }
    drop(state);
    let (key, identity) = first.unwrap();
    assert!(matches!(
        table.admit(key.clone(), identity).unwrap(),
        CallAdmission::Evicted
    ));
    let changed = RequestIdentity::model(&model("body-0", "local", "fixture", "different"));
    assert!(matches!(
        table.admit(key, changed).unwrap(),
        CallAdmission::Conflict
    ));
}

#[test]
fn eviction_uses_completion_order_instead_of_admission_or_key_order() {
    let table = CallTable::<()>::new();
    let attempt = AttemptId::new();
    let identity = RequestIdentity::model(&model("identity", "local", "fixture", "prompt"));
    let slow_key = (attempt, CallId::parse("a-admitted-first").unwrap());
    let slow = occupy(&table, slow_key.clone(), identity);
    let mut first_completed = None;
    for index in 0..MAX_RETAINED_RESULTS {
        let key = (attempt, CallId::parse(format!("z-{index:04}")).unwrap());
        first_completed.get_or_insert_with(|| key.clone());
        occupy(&table, key, identity).complete(()).unwrap();
    }
    slow.complete(()).unwrap();
    assert!(matches!(
        table.admit(first_completed.unwrap(), identity).unwrap(),
        CallAdmission::Evicted
    ));
    assert!(matches!(
        table.admit(slow_key.clone(), identity).unwrap(),
        CallAdmission::Replay(())
    ));
    assert_eq!(
        table.state.lock().unwrap().completed.back(),
        Some(&slow_key)
    );
}

fn assert_conflicts(original: RequestIdentity, changed: impl IntoIterator<Item = RequestIdentity>) {
    let table = CallTable::<()>::new();
    let key = (AttemptId::new(), CallId::parse("identity").unwrap());
    occupy(&table, key.clone(), original).complete(()).unwrap();
    assert!(matches!(
        table.admit(key.clone(), original).unwrap(),
        CallAdmission::Replay(())
    ));
    for identity in changed {
        assert!(matches!(
            table.admit(key.clone(), identity).unwrap(),
            CallAdmission::Conflict
        ));
    }
}

#[test]
fn request_identity_covers_all_fields_and_separates_field_boundaries() {
    let original = RequestIdentity::tool(&tool("call", "web.fetch", json!({"a": 1})));
    assert_conflicts(
        original,
        [
            RequestIdentity::tool(&tool("other", "web.fetch", json!({"a": 1}))),
            RequestIdentity::tool(&tool("call", "db.query", json!({"a": 1}))),
            RequestIdentity::tool(&tool("call", "web.fetch", json!({"a": 2}))),
            RequestIdentity::model(&model("call", "web.fetch", "", "{\"a\":1}")),
        ],
    );
    let original = RequestIdentity::model(&model("call", "ab", "c", "prompt"));
    assert_conflicts(
        original,
        [
            RequestIdentity::model(&model("other", "ab", "c", "prompt")),
            RequestIdentity::model(&model("call", "other", "c", "prompt")),
            RequestIdentity::model(&model("call", "ab", "other", "prompt")),
            RequestIdentity::model(&model("call", "ab", "c", "other")),
            RequestIdentity::model(&model("call", "a", "bc", "prompt")),
        ],
    );
}

#[test]
fn json_identity_obeys_nested_object_equality_and_array_order() {
    let left: Value =
        serde_json::from_str(r#"{"z":[{"a":1,"b":2}],"a":{"x":true,"y":null}}"#).unwrap();
    let right: Value =
        serde_json::from_str(r#"{"a":{"y":null,"x":true},"z":[{"b":2,"a":1}]}"#).unwrap();
    assert_eq!(left, right);
    let identity = |value| RequestIdentity::tool(&tool("call", "web.fetch", value));
    assert_eq!(identity(left), identity(right));
    assert_ne!(identity(json!([1, 2])), identity(json!([2, 1])));
    let values = [
        json!(null),
        json!(false),
        json!("false"),
        json!([]),
        json!({}),
        json!(0),
    ];
    for (index, left) in values.iter().enumerate() {
        for right in &values[index + 1..] {
            assert_ne!(identity(left.clone()), identity(right.clone()));
        }
    }
}

#[test]
fn numeric_identity_matches_value_equality_including_signed_zero() {
    let values: Vec<Value> = [
        "0",
        "-0",
        "0.0",
        "-0.0",
        "1",
        "1.0",
        "1e0",
        "-1",
        "-1.0",
        "9007199254740992",
        "9007199254740993",
        "18446744073709551615",
        "9223372036854775808",
        "-9223372036854775808",
        "1e-308",
        "1e308",
    ]
    .into_iter()
    .map(|number| serde_json::from_str(number).unwrap())
    .collect();
    for left in &values {
        for right in &values {
            assert_eq!(
                RequestIdentity::tool(&tool("call", "web.fetch", left.clone()))
                    == RequestIdentity::tool(&tool("call", "web.fetch", right.clone())),
                left == right,
                "number equality changed for {left} and {right}"
            );
        }
    }
    assert_ne!(json!(1), json!(1.0));
}
