//! `/connect/v1/*` 在关停开始后回 503 并记 server_stopping，不再接新请求（同 `/read/*`，T-184）。
use crate::{options::ServeOptions, serve, state::AppState};
use serde_json::{Value, json};
use std::{
    fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::AgentStore;
use tempfile::TempDir;

include!("../mcp_contract_support/fixture.rs");

#[tokio::test]
async fn connector_requests_are_refused_once_stopping_begins() {
    let f = fixture().await;
    let list = f.base.replace("/mcp", "/connect/v1/board");
    let get = || f.client.get(&list).bearer_auth(&f.token).send();
    // 没接策略平面就没有任何连接器权限；这里只看它走到了裁决那一步。
    assert_eq!(get().await.unwrap().status(), 403, "关停前照常裁决");
    f.state.lifecycle.stop();
    let after = get().await.unwrap();
    assert_eq!(after.status(), 503);
    assert_eq!(after.headers()["studiolo-connect"], "1");
    assert_eq!(after.headers()["retry-after"], "30");
    let body = serde_json::from_slice::<Value>(&after.bytes().await.unwrap()).unwrap();
    assert_eq!(body["code"], "stopping");
    let reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| event.event().payload().as_value()["reason"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        reasons,
        [json!("policy_not_configured"), json!("server_stopping")]
    );
}
