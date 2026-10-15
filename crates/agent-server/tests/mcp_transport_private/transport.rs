//! 真 rmcp manager 与 loopback HTTP；小容量只在私有测试构造器中注入。
use super::{
    assemble,
    manager::{Manager, REQUEST_BINDING, Reservation},
    pool::{Limits, Pool},
};
use crate::{options::ServeOptions, serve, state::AppState};
use rmcp::transport::streamable_http_server::session::{
    SessionManager, local::LocalSessionManager,
};
use serde_json::{Value, json};
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::AgentStore;
use tempfile::TempDir;

include!("../mcp_contract_support/fixture.rs");
include!("lifecycle.rs");
include!("late_initialize.rs");
include!("backpressure.rs");
include!("streams.rs");
include!("replay.rs");
include!("expiry.rs");
include!("dispatch.rs");

async fn bounded(limits: Limits) -> (Fixture, Arc<Manager>) {
    let mut manager = None;
    let fixture = fixture_with_router(|app| {
        let (router, value) = assemble(app, limits);
        manager = Some(value);
        router
    })
    .await;
    (fixture, manager.unwrap())
}

async fn empty(manager: &Manager) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !manager.pool.all().is_empty() || !manager.inner.sessions.read().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("实际 worker 与预占应一同清理");
}

fn initialization() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}
    }})
}

#[tokio::test]
async fn delete_releases_actual_worker_capacity_and_new_handshake_can_recover() {
    let (f, manager) = bounded(Limits {
        total: 1,
        per_type: 1,
        ..Limits::default()
    })
    .await;
    let (session, _) = f.initialize(&f.token).await;
    let full = f
        .request(Some(&f.token), None, initialization())
        .send()
        .await
        .unwrap();
    assert_eq!(full.status(), 429);
    assert_eq!(full.headers()["retry-after"], "60");
    let deleted = f
        .client
        .delete(&f.base)
        .bearer_auth(&f.token)
        .header("Mcp-Session-Id", session)
        .send()
        .await
        .unwrap();
    assert!(deleted.status().is_success());
    empty(&manager).await;
    let (new, _) = f.initialize(&f.token).await;
    assert!(manager.pool.find(&new).is_some());
    assert_eq!(manager.inner.sessions.read().await.len(), 1);
}

#[tokio::test]
async fn failed_protocol_starts_do_not_leak_capacity_or_bypass_initialization_rate() {
    let (f, manager) = bounded(Limits {
        total: 1,
        starts_per_minute: 2,
        ..Limits::default()
    })
    .await;
    for method in ["tools/list", "server/discover"] {
        let rejected = f
            .request(
                Some(&f.token),
                None,
                json!({"jsonrpc":"2.0","id":1,"method":method}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), 400);
        empty(&manager).await;
    }
    let full = f
        .request(Some(&f.token), None, initialization())
        .send()
        .await
        .unwrap();
    assert_eq!(full.status(), 429);
    assert!(manager.pool.all().is_empty());
    assert!(
        f.ledger()
            .query(&AuditFilter::default().with_action("agent_instance.registered"))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn audit_failure_is_bounded_and_never_delivers_content() {
    let (f, manager) = bounded(Limits {
        total: 1,
        ..Limits::default()
    })
    .await;
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    let (session, response) = f.initialize(&f.token).await;
    assert!(
        response["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("审计不可用")
    );
    assert!(!response.to_string().contains("Alpha"));
    let failed = f
        .call(
            &f.token,
            &session,
            2,
            "read_note",
            json!({"path":"项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(is_error(&failed));
    assert!(tool_text(&failed).is_empty());
    assert_eq!(
        f.request(Some(&f.token), None, initialization())
            .send()
            .await
            .unwrap()
            .status(),
        429
    );
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    f.client
        .delete(&f.base)
        .bearer_auth(&f.token)
        .header("Mcp-Session-Id", session)
        .send()
        .await
        .unwrap();
    empty(&manager).await;
    let (_, recovered) = f.initialize(&f.token).await;
    assert!(
        recovered["result"]["instructions"]
            .as_str()
            .unwrap()
            .starts_with("# Studiolo 路牌")
    );
    assert!(f.state.degraded().is_none());
    assert_eq!(f.store().outages().unwrap().len(), 1);
}

#[tokio::test]
async fn existing_default_router_still_supplies_tools_through_the_bound_session() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    assert!(!is_error(
        &f.call(&f.token, &session, 2, "handshake", json!({})).await
    ));
}
