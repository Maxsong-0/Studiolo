use super::*;
const READS: &str = "/api/v1/agents/reads/query";
fn body() -> Value {
    json!({"outcome":"read","agent_type":null,"instance_id":null,"since":null,"until":null,"path_prefix":null,"before":null})
}
#[tokio::test]
async fn reads_query_uses_owner_csrf_and_closed_verified_projection() {
    let mut f = Fixture::with_identities(true, true).await;
    let body = body().to_string();
    f.send(f.request("POST", READS, None, &body), 401).await;
    let login = f.issue().await;
    f.send(
        f.request("POST", READS, Some(&login), &" ".repeat(4097)),
        413,
    )
    .await;
    f.send(f.request("GET", "/api/v1/agents", Some(&login), "{}"), 413)
        .await;
    f.send(f.request("GET", READS, Some(&login), ""), 405).await;
    f.send(
        f.request("POST", &format!("{READS}?x=1"), Some(&login), &body),
        400,
    )
    .await;
    let mut request = f.request("POST", READS, Some(&login), &body);
    request.headers_mut().remove("x-csrf-token");
    f.send(request, 403).await;
    for invalid in [
        json!({"outcome":"read","extra":true}),
        json!({"outcome":"read","since":"yesterday"}),
        json!({"outcome":"denied","path_prefix":"x"}),
        json!({"outcome":"read","before":0}),
    ] {
        f.send(
            f.request("POST", READS, Some(&login), &invalid.to_string()),
            400,
        )
        .await;
    }
    let ledger = SqliteAuditLedger::open_existing(&f.root.join("agents/audit")).unwrap();
    let actor = studiolo_supply::audit::Actor::instance("claude", "01K00000000000000000000001");
    ledger
        .append(
            studiolo_supply::audit::content_read(actor, "项目/A/a.md", "v1", "read_note").unwrap(),
        )
        .unwrap();
    ledger
        .append(
            studiolo_supply::audit::action_denied(actor, "read_note", "capability_denied").unwrap(),
        )
        .unwrap();
    let result = f
        .send(f.request("POST", READS, Some(&login), &body), 200)
        .await
        .json();
    assert_eq!(result.as_object().unwrap().len(), 6);
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    assert_eq!(result["records"][0].as_object().unwrap().len(), 8);
    assert_eq!(result["records"][0]["path"], "项目/A/a.md");
    let denied = f
        .send(
            f.request("POST", READS, Some(&login), r#"{"outcome":"denied"}"#),
            200,
        )
        .await
        .json();
    assert!(denied["records"][0]["path"].is_null());
    assert_eq!(denied["records"][0]["reason"], "capability_denied");
    let raw = rusqlite::Connection::open(f.root.join("agents/audit/audit.sqlite3")).unwrap();
    raw.execute(
        "UPDATE audit_events SET record_json='{}' WHERE offset=2",
        [],
    )
    .unwrap();
    let failed = f
        .send(f.request("POST", READS, Some(&login), &body), 503)
        .await;
    assert_eq!(failed.json()["code"], "reads_unavailable");
    assert!(!String::from_utf8_lossy(&failed.body).contains(f.root.to_str().unwrap()));
    f.stop().await;
}
#[tokio::test]
async fn reads_without_audit_plane_are_not_an_empty_success() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let missing = f
        .send(
            f.request("POST", READS, Some(&login), &body().to_string()),
            404,
        )
        .await
        .json();
    assert_eq!(missing["code"], "reads_plane_absent");
    f.stop().await;
}
