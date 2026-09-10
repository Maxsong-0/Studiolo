//! 真实 loopback 契约：raw JSON-RPC over Streamable HTTP（reqwest 0.12，不引入第二个 reqwest）。
//! 覆盖：401、初始化路牌与硬预算、tools/list、read_note 正反、list_entries、search、吊销即刻生效、
//! 审计先于返回与读取索引四维查询、审计不可用时的降级握手。

use std::{
    fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::{AgentStore, HARD_BUDGET_TOKENS, ReadQuery, count_tokens};
use tempfile::TempDir;

// 机械拆分夹具与最长契约，保留原有顶层测试名。
include!("mcp_contract_support/fixture.rs");
include!("mcp_cases/read_only.rs");

#[tokio::test]
async fn unauthenticated_requests_are_rejected_before_any_content_and_audited() {
    let f = fixture().await;
    for token in [None, Some("sag1_wrong"), Some("")] {
        let response = f
            .request(token, None, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "x", "version": "0"}}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert!(response.headers().get("www-authenticate").is_some());
        assert!(response.text().await.unwrap().is_empty());
    }
    let denied = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap();
    assert_eq!(denied.len(), 3);
    assert_eq!(
        denied[0].event().payload().as_value()["reason"],
        "unauthenticated"
    );
    assert!(denied[0].event().payload().as_value()["agent_type"].is_null());
}

#[tokio::test]
async fn audit_unavailable_degrades_the_handshake_without_any_vault_content() {
    let f = fixture().await;
    // 破坏审计私有目录权限：账本存储复核失败 → 追加失败 → 降级握手。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    assert!(
        f.state
            .ledger
            .append(
                studiolo_supply::audit::handshake_degraded(
                    studiolo_supply::audit::Actor::anonymous(),
                    "probe"
                )
                .unwrap()
            )
            .is_err(),
        "ledger storage check must fail closed after the directory mode changed"
    );
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("审计不可用：outage "));
    // 路牌六项一项不给：没有项目状态、待办、准则、入口，也没有任何 vault 内容。
    for leaked in [
        "# Studiolo 路牌",
        "## 1 我是谁",
        "Alpha",
        "准则",
        "永远说中文",
        "入口：",
        "tools：",
    ] {
        assert!(
            !instructions.contains(leaked),
            "degraded handshake leaked {leaked}: {instructions}"
        );
    }
    let denied = f
        .call(
            &f.token,
            &session,
            2,
            "read_note",
            json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(is_error(&denied));
    assert!(!denied.to_string().contains("Alpha · 总览"));
    let outage_id = instructions
        .trim_start_matches("审计不可用：outage ")
        .split('。')
        .next()
        .unwrap()
        .to_owned();
    assert!(f.state.degraded().is_some());
    // 恢复后：新握手正常，outage 记录已结束且带拒绝计数。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    let (_, recovered) = f.initialize(&f.token).await;
    assert!(
        recovered["result"]["instructions"]
            .as_str()
            .unwrap()
            .starts_with("# Studiolo 路牌")
    );
    assert!(f.state.degraded().is_none());
    let outages = f.store().outages().unwrap();
    assert_eq!(outages.len(), 1);
    assert_eq!(outages[0].0, outage_id);
    assert!(outages[0].2.is_some(), "outage ended");
    assert!(
        outages[0].3 >= 2,
        "instance registration + read were denied: {}",
        outages[0].3
    );
}
