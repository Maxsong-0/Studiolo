//! 固定合法审批文档；**不含任何收件地址或凭据**。

use serde_json::{Value, json};

pub fn document() -> Value {
    json!({
        "schema_version": 1,
        "enabled": true,
        "state_root": "/private/studiolo/observability",
        "recovery_point_budget_seconds": 90_000,
        "owners": ["backup-worker", "sync-transport"],
        "deadman": {
            "period_seconds": 86_400,
            "debounce_seconds": 600,
            "cooldown_seconds": 21_600,
            "max_suppressed": 3
        },
        "disk": {
            "minimum_free_bytes": 10_737_418_240_u64,
            "warn_free_bytes": 32_212_254_720_u64
        },
        "sync": {
            "max_objects": 50_000,
            "max_bytes": 107_374_182_400_u64,
            "warn_objects": 40_000,
            "warn_bytes": 85_899_345_920_u64
        },
        "dispatch": { "max_attempts": 3, "dedupe_capacity": 64 }
    })
}

pub fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("固定夹具必须可序列化")
}

/// 在固定文档上改一个字段再序列化。
pub fn with(mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = document();
    mutate(&mut value);
    bytes(&value)
}
