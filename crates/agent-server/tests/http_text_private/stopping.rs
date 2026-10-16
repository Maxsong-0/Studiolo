//! `/read/*` 在关停开始后回 503 并记 server_stopping，不再接新请求（T-184，BUG-033）。
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
async fn reads_are_refused_once_stopping_begins() {
    let f = fixture().await;
    let handshake = f.base.replace("/mcp", "/read/handshake");
    let before = f
        .client
        .get(&handshake)
        .bearer_auth(&f.token)
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 200, "关停前照常握手");
    f.state.lifecycle.stop();
    let after = f
        .client
        .get(&handshake)
        .bearer_auth(&f.token)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 503);
    assert_eq!(after.text().await.unwrap(), "服务正在关停，请稍后重试");
    let reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| event.event().payload().as_value()["reason"].clone())
        .collect::<Vec<_>>();
    assert_eq!(reasons, [json!("server_stopping")]);
}
