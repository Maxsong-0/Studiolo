//! 行为契约：显式启用的审批如实报告被批准的数值，不多给任何能力。

use super::fixture::{bytes, document};
use studiolo_host_config::alerting::AlertingApproval;

#[test]
fn 显式启用的审批如实报告全部预算与组件角色() {
    let approval = AlertingApproval::parse(&bytes(&document())).expect("固定夹具必须合法");

    assert_eq!(
        approval.state_root(),
        std::path::Path::new("/private/studiolo/observability")
    );
    assert_eq!(approval.recovery_point_budget_seconds(), 90_000);
    assert_eq!(approval.owners(), ["backup-worker", "sync-transport"]);
    assert_eq!(approval.deadman().period_seconds(), 86_400);
    assert_eq!(approval.deadman().debounce_seconds(), 600);
    assert_eq!(approval.deadman().cooldown_seconds(), 21_600);
    assert_eq!(approval.deadman().max_suppressed(), 3);
    assert_eq!(approval.disk().minimum_free_bytes(), 10_737_418_240);
    assert_eq!(approval.sync().max_objects(), 50_000);
    assert_eq!(approval.dispatch().max_attempts(), 3);
    assert_ne!(approval.digest(), [0_u8; 32]);
}

#[test]
fn 内容不同的两份审批得到不同摘要() {
    let left = AlertingApproval::parse(&bytes(&document())).expect("固定夹具必须合法");
    let right = AlertingApproval::parse(&super::fixture::with(|value| {
        value["recovery_point_budget_seconds"] = serde_json::json!(90_001);
    }))
    .expect("改动后的夹具仍应合法");

    assert_ne!(left.digest(), right.digest());
}
