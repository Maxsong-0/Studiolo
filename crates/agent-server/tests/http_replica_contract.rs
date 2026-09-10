//! HTTP 兜底逐请求复核副本；与 MCP 一样保留真实来源时间并失败关闭。
#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;
#[allow(dead_code)]
#[path = "http_text/support.rs"]
mod support;

use replica_support::{FIRST, ReplicaFixture, SECOND, write};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_agent_server::options::ServeOptions;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};
use studiolo_supply::{HARD_BUDGET_TOKENS, count_tokens};
use support::{HttpServer, assert_headers, denied, header};

async fn fixture(age_ms: i64) -> (ReplicaFixture, HttpServer) {
    let mut replica = ReplicaFixture::new();
    replica.observed = jiff::Timestamp::now().as_millisecond() - age_ms;
    replica.publish(FIRST, "oldmarker", replica.observed);
    let server = support::spawn(ServeOptions {
        vault: None,
        replica_root: Some(replica.root.clone()),
        replica_max_age_secs: 300,
        allow: vec!["project".into()],
        data: replica.temp.path().join("agent-data"),
        port: 0,
    })
    .await;
    (replica, server)
}

const ROUTES: [&str; 4] = [
    "/read/handshake",
    "/read/entries?project=project",
    "/read/note?path=project%2Fnote.md",
    "/read/search?project=project&text=marker",
];

#[tokio::test]
async fn all_http_reads_follow_verified_updates_without_reusing_old_file_content() {
    let (replica, f) = fixture(10_000).await;
    let (session, instance, initial) = f.handshake().await;
    assert!(initial.contains("状态 ready") && initial.contains("oldmarker"));
    assert!(count_tokens(&initial) <= HARD_BUDGET_TOKENS);
    replica.publish(SECOND, "newmarker", replica.observed + 1000);
    for route in ROUTES {
        let response = f.get(route, Some(&session)).send().await.unwrap();
        assert_eq!(response.status(), 200, "{route}");
        assert_headers(&response, true);
        assert_eq!(header(&response, "studiolo-instance-id"), instance);
        let text = response.text().await.unwrap();
        assert!(
            text.contains(&format!("generation={SECOND}")),
            "{route}: {text}"
        );
        assert!(!text.contains("oldmarker"));
        if !route.starts_with("/read/entries") {
            assert!(text.contains("newmarker"), "{route}: {text}");
        }
    }
    let reads = f
        .store()
        .query_reads(&studiolo_supply::ReadQuery {
            instance_id: Some(instance),
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    for tool in ["read_note", "list_entries", "search"] {
        assert!(reads.iter().any(|read| read.tool == tool));
    }
}

#[tokio::test]
async fn stale_and_failed_sync_remain_explicit_and_never_forge_source_confirmation() {
    let (replica, f) = fixture(400_000).await;
    let (session, _, initial) = f.handshake().await;
    assert!(initial.contains("状态 stale") && initial.contains("oldmarker"));
    let mut state = replica.state();
    let observed = state["receipt"]["source_observed_at_ms"].as_i64().unwrap();
    state["status"] = json!("failed");
    state["last_error"] = json!("transfer_failed");
    state["last_attempt_at_ms"] = json!(jiff::Timestamp::now().as_millisecond());
    replica.write_state(&state);
    for route in ROUTES {
        let response = f.get(route, Some(&session)).send().await.unwrap();
        assert_eq!(response.status(), 200, "{route}");
        assert_headers(&response, true);
        let text = response.text().await.unwrap();
        assert!(
            text.contains("sync_failed") && text.contains("stale=true"),
            "{route}: {text}"
        );
        assert!(text.contains(&format!("generation={FIRST}")));
        assert!(text.contains(&format!("source_observed_at_ms={observed}")));
        if route == "/read/handshake" {
            assert!(count_tokens(&text) <= HARD_BUDGET_TOKENS);
        }
    }
}

#[tokio::test]
async fn drift_and_invalid_control_refuse_content_and_audit_outage_has_precedence() {
    let (replica, f) = fixture(10_000).await;
    let (session, _, _) = f.handshake().await;
    write(&replica.file("project/note.md"), b"# injected secret");
    for route in ROUTES {
        let body = denied(f.get(route, Some(&session)).send().await.unwrap(), 503).await;
        assert!(body.contains("不可用"), "{route}: {body}");
        assert!(!body.contains("injected secret") && !body.contains("oldmarker"));
    }
    let events = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap();
    assert_eq!(events.len(), 4);
    write(&replica.root.join("control/state.json"), b"{}");
    for session in [None, Some(session.as_str())] {
        let body = denied(f.get("/read/handshake", session).send().await.unwrap(), 503).await;
        assert!(body.contains("不可用"), "{body}");
        assert!(!body.contains("oldmarker") && !body.contains("# Studiolo 路牌"));
    }
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    let body = denied(
        f.get("/read/handshake", Some(&session))
            .send()
            .await
            .unwrap(),
        503,
    )
    .await;
    assert!(body.contains("审计不可用") && body.contains("outage"));
    assert!(!body.contains("replica_invalid") && !body.contains("injected secret"));
}
