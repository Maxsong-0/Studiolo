//! 身份库读不出来时黑板如实报暂不可用（BUG-032）：私有入口回 503、不回 409，
//! 不把读错说成「实例无效」；恢复后同一实例照常。故障只靠改库文件权限、给实例表改名
//! 再改回来注入，不加生产测试钩子。
use super::{PROJECT, TestBoard};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};

const UNAVAILABLE: &str = "身份库暂不可用，请稍后重试";
/// 格式合法、从未登记的实例号。
const UNKNOWN_INSTANCE: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn database(f: &TestBoard) -> PathBuf {
    f.writer.http.root.join("agent/agent.sqlite3")
}

/// 0640 让身份库的私有存储复核失败，之后每次读都出错；改回 0600 即恢复。
fn set_mode(f: &TestBoard, mode: u32) {
    fs::set_permissions(database(f), fs::Permissions::from_mode(mode)).unwrap();
}

/// 只让实例表读不出来：类型表照常，`authorize` 先过类型检查，再在实例链这一步出错。
fn rename_instances(f: &TestBoard, from: &str, to: &str) {
    let connection = rusqlite::Connection::open(database(f)).unwrap();
    // 服务端连接可能还没放开读事务：等一会儿再改表，不因一时忙碌偶发失败。
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    connection
        .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
        .unwrap();
}

/// 私有入口暂不可用的答复：固定 503 正文，不回显内部原因。
fn assert_unavailable(body: &Value) {
    assert_eq!(
        body,
        &json!({"code": "board_unavailable", "error": "黑板暂不可用，请重试。"})
    );
}

/// 最近一条黑板请求结果事件记下的原因。
fn last_outcome_reason(f: &TestBoard) -> String {
    let ledger = SqliteAuditLedger::open_existing(&f.writer.http.audit).unwrap();
    let records = ledger
        .query(
            &AuditFilter::default()
                .with_action("board.request")
                .with_phase(AuditPhase::Outcome),
        )
        .unwrap();
    records.last().unwrap().event().payload().as_value()["reason"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn unreadable_identity_store_answers_unavailable_and_recovers() {
    let f = TestBoard::new().await;
    f.call("list", json!({}), 200).await;
    set_mode(&f, 0o640);
    assert_unavailable(&f.call("list", json!({}), 503).await);
    let create = json!({"request_id": "identity-outage", "spec": f.spec()});
    assert_unavailable(&f.call("create", create.clone(), 503).await);
    assert_eq!(last_outcome_reason(&f), UNAVAILABLE);
    set_mode(&f, 0o600);
    // 暂不可用期间什么都没写：恢复后板上仍是空的，同一请求号照常创建。
    assert_eq!(f.call("list", json!({}), 200).await, json!([]));
    f.call("create", create, 200).await;
    assert_eq!(
        f.call("list", json!({}), 200)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.stop().await;
}

#[tokio::test]
async fn unreadable_instance_table_is_not_reported_as_an_invalid_instance() {
    let f = TestBoard::new().await;
    // 库正常、实例确实不存在：仍是确定拒绝，文案不变。
    let denied = f
        .call_as("claude", UNKNOWN_INSTANCE, PROJECT, "list", json!({}), 409)
        .await;
    assert_eq!(denied, json!({"error": "实例无效"}));
    assert_eq!(last_outcome_reason(&f), "实例无效");
    rename_instances(&f, "agent_instance", "agent_instance_hidden");
    assert_unavailable(&f.call("list", json!({}), 503).await);
    assert_eq!(last_outcome_reason(&f), UNAVAILABLE);
    rename_instances(&f, "agent_instance_hidden", "agent_instance");
    f.call("list", json!({}), 200).await;
    f.stop().await;
}
