//! 失败契约：默认关闭，越界/未知字段/自相矛盾一律拒绝，且不产生部分能力。

use super::fixture::with;
use studiolo_host_config::{ConfigError, alerting::AlertingApproval};

fn rejected(bytes: &[u8]) {
    assert_eq!(AlertingApproval::parse(bytes).err(), Some(ConfigError));
}

#[test]
fn 未显式启用时不产生任何能力() {
    rejected(&with(|value| value["enabled"] = serde_json::json!(false)));
}

#[test]
fn schema版本未知时拒绝() {
    rejected(&with(|value| {
        value["schema_version"] = serde_json::json!(2)
    }));
}

#[test]
fn 形似收件地址的多余字段被未知字段规则直接拒绝而不是静默忽略() {
    rejected(&with(|value| {
        value["alert_to"] = serde_json::json!("someone@example.invalid");
    }));
    rejected(&with(|value| {
        value["smtp_url"] = serde_json::json!("smtps://user:pass@host");
    }));
}

#[test]
fn 状态目录不是绝对规范路径时拒绝() {
    rejected(&with(|value| {
        value["state_root"] = serde_json::json!("relative/dir")
    }));
    rejected(&with(|value| {
        value["state_root"] = serde_json::json!("/a/../b")
    }));
}

#[test]
fn 去抖窗口大于约定周期时拒绝() {
    rejected(&with(|value| {
        value["deadman"]["debounce_seconds"] = serde_json::json!(86_401);
    }));
}

#[test]
fn 压制上限为零时拒绝因为那等于把连续故障永远压住() {
    rejected(&with(|value| {
        value["deadman"]["max_suppressed"] = serde_json::json!(0);
    }));
}

#[test]
fn 磁盘预警水位比阻断下限更紧时拒绝() {
    rejected(&with(|value| {
        value["disk"]["warn_free_bytes"] = serde_json::json!(1_u64);
    }));
}

#[test]
fn 磁盘阻断下限为零时拒绝() {
    rejected(&with(|value| {
        value["disk"]["minimum_free_bytes"] = serde_json::json!(0_u64);
    }));
}

#[test]
fn sync预警水位超过上限时拒绝() {
    rejected(&with(|value| {
        value["sync"]["warn_objects"] = serde_json::json!(50_001_u64);
    }));
}

#[test]
fn 调度重试次数为零或超过上限时拒绝() {
    rejected(&with(|value| {
        value["dispatch"]["max_attempts"] = serde_json::json!(0);
    }));
    rejected(&with(|value| {
        value["dispatch"]["max_attempts"] = serde_json::json!(17);
    }));
}

#[test]
fn 组件角色为空重复或含非法字符时拒绝() {
    rejected(&with(|value| value["owners"] = serde_json::json!([])));
    rejected(&with(|value| {
        value["owners"] = serde_json::json!(["backup-worker", "backup-worker"]);
    }));
    rejected(&with(|value| {
        value["owners"] = serde_json::json!(["Owner@Example"]);
    }));
}

#[test]
fn 恢复点预算越界时拒绝() {
    rejected(&with(|value| {
        value["recovery_point_budget_seconds"] = serde_json::json!(59_u64);
    }));
    rejected(&with(|value| {
        value["recovery_point_budget_seconds"] = serde_json::json!(90_u64 * 86_400 + 1);
    }));
}

#[test]
fn 非对象输入尾随内容与超预算字节一律拒绝() {
    rejected(b"[]");
    rejected(b"{\"schema_version\":1} {}");
    rejected(&vec![b' '; 8 * 1024 + 1]);
    rejected(b"");
}
