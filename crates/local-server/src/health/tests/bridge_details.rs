use super::{HealthBody, complete};
use serde_json::{Value, json};
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::BudgetUsage;

fn body(used: u64, recorded: i64) -> Value {
    let mut snapshot = complete(100_000);
    snapshot.manual_sync_quota = Some(ManualQuota {
        account_limit_bytes: 1000,
        used_bytes_manual: used,
        recorded_at_unix: recorded,
    });
    serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap()
}
#[test]
fn manual_account_threshold_is_separate_from_mirror_usage_and_time_is_required() {
    for (used, recorded, state, reason) in [
        (699, 99_999, "normal", "within"),
        (700, 99_999, "degraded", "warning"),
        (1001, 99_999, "degraded", "exceeded"),
        (100, 13_599, "unknown", "stale"),
        (100, 100_001, "unknown", "clock"),
        (100, -1, "unknown", "clock"),
    ] {
        let body = body(used, recorded);
        assert_eq!(body["manual_sync_quota"]["state"], state);
        assert_eq!(body["manual_sync_quota"]["reason"], reason);
        assert_eq!(
            body["manual_sync_quota"]["used_bytes_manual"],
            used.to_string()
        );
        assert_eq!(body["bridge"]["mirror_bytes"], "4096");
        if state != "normal" {
            assert_ne!(body["overall"], "normal");
        }
    }
}
#[test]
fn bridge_counts_are_lossless_closed_and_survive_degraded_summary() {
    let mut snapshot = complete(100_000);
    let bridge = snapshot.bridge.as_mut().unwrap();
    bridge.fused = Some(true);
    bridge.unknown_jobs = Some(u64::MAX);
    bridge.mirror_bytes = Some(u64::MAX);
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert_eq!(body["bridge"]["unknown_jobs"], u64::MAX.to_string());
    assert_eq!(body["bridge"]["deletes_blocked"], "0");
    assert_eq!(body["bridge"].as_object().unwrap().len(), 14);
    assert_eq!(body["manual_sync_quota"].as_object().unwrap().len(), 8);
    assert_eq!(body.as_object().unwrap().len(), 6);
    assert_eq!(body["overall"], "degraded");
    assert_eq!(body["bridge"]["fused"], true);
}
/// 路径用量（BUG-047）按十进制字符串传两个数；还没算到过就是 `null`，超上限时整页判降级。
#[test]
fn path_usage_is_two_lossless_counts_or_null() {
    let mut snapshot = complete(100_000);
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert!(body["bridge"]["path_usage"].is_null());

    snapshot.bridge.as_mut().unwrap().path_usage = Some(BudgetUsage {
        used: u64::MAX,
        budget: 16_384,
    });
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert_eq!(
        body["bridge"]["path_usage"],
        json!({"used": u64::MAX.to_string(), "budget": "16384"})
    );
    assert_eq!(body["overall"], "degraded");
}
/// 比对内容用量同样是两个十进制字符串；还没算到过就是 `null`，超上限时整页判降级。
#[test]
fn byte_usage_is_two_lossless_counts_or_null() {
    let mut snapshot = complete(100_000);
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert!(body["bridge"]["byte_usage"].is_null());

    snapshot.bridge.as_mut().unwrap().byte_usage = Some(BudgetUsage {
        used: u64::MAX,
        budget: 4_294_967_296,
    });
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert_eq!(
        body["bridge"]["byte_usage"],
        json!({"used": u64::MAX.to_string(), "budget": "4294967296"})
    );
    assert!(body["bridge"]["path_usage"].is_null());
    assert_eq!(body["overall"], "degraded");
}
#[test]
fn missing_probe_or_manual_record_does_not_create_normal_evidence() {
    let snapshot = super::EvidenceSnapshot::unknown(100_000);
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert!(body["bridge"].is_null());
    assert!(body["manual_sync_quota"].is_null());
    assert_eq!(body["overall"], "unknown");
    assert!(
        body["sections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|section| section["state"] == "unknown")
    );
}
#[test]
fn maximum_manual_counts_do_not_overflow_the_seventy_percent_comparison() {
    let mut snapshot = complete(100_000);
    snapshot.manual_sync_quota = Some(ManualQuota {
        account_limit_bytes: u64::MAX,
        used_bytes_manual: u64::MAX - 1,
        recorded_at_unix: 99_999,
    });
    let body = serde_json::to_value(HealthBody::from_snapshot(&snapshot)).unwrap();
    assert_eq!(body["manual_sync_quota"]["reason"], "warning");
}
