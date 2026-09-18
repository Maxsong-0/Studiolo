//! 恢复契约：被拒绝的审批不留部分状态，改回合法值后立即重新可用。

use super::fixture::{bytes, document, with};
use studiolo_host_config::{ConfigError, alerting::AlertingApproval};

#[test]
fn 一次被拒绝之后用同一份合法文档立即重新解析成功() {
    assert_eq!(
        AlertingApproval::parse(&with(|value| value["enabled"] = serde_json::json!(false))).err(),
        Some(ConfigError)
    );

    let approval = AlertingApproval::parse(&bytes(&document())).expect("固定夹具必须合法");

    assert_eq!(approval.deadman().period_seconds(), 86_400);
}

#[test]
fn 把越界预算改回范围内之后审批重新可用且摘要随内容变化() {
    let broken = with(|value| value["dispatch"]["max_attempts"] = serde_json::json!(0));
    assert_eq!(AlertingApproval::parse(&broken).err(), Some(ConfigError));

    let repaired = with(|value| value["dispatch"]["max_attempts"] = serde_json::json!(5));
    let approval = AlertingApproval::parse(&repaired).expect("改回范围内后必须合法");

    assert_eq!(approval.dispatch().max_attempts(), 5);
    assert_ne!(
        approval.digest(),
        AlertingApproval::parse(&bytes(&document()))
            .expect("固定夹具必须合法")
            .digest()
    );
}
