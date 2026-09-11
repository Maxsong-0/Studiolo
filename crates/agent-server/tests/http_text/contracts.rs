use super::support::{assert_headers, fixture, header};
use serde_json::json;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};
use studiolo_supply::{HARD_BUDGET_TOKENS, ReadQuery, count_tokens};

#[tokio::test]
async fn four_http_operations_are_budgeted_scoped_and_audited_under_one_instance() {
    let (_temp, f) = fixture().await;
    let (session, instance, signpost) = f.handshake().await;
    assert!(signpost.starts_with("# Studiolo 路牌"));
    assert!(signpost.contains(&format!("类型 claude · 实例 {instance}")));
    assert!(signpost.contains("### 项目/Alpha\n当前版本：v0.3"));
    assert!(signpost.contains("T-037 HTTP 兜底"));
    assert!(signpost.contains("永远说中文"));
    for route in [
        "/read/handshake",
        "/read/entries",
        "/read/note",
        "/read/search",
    ] {
        assert!(signpost.contains(route), "missing HTTP entry {route}");
    }
    assert!(!signpost.contains("tools："));
    assert!(!signpost.contains("秘密") && !signpost.contains("Beta"));
    assert!(count_tokens(&signpost) <= HARD_BUDGET_TOKENS);

    for (route, params, expected) in [
        (
            "/read/note",
            vec![("path", "项目/Alpha/00-总览/INDEX.md")],
            "# Alpha · 总览",
        ),
        (
            "/read/entries",
            vec![("project", "项目/Alpha")],
            "note\t项目/Alpha/00-总览/INDEX.md\t",
        ),
        (
            "/read/search",
            vec![
                ("project", "项目/Alpha"),
                ("text", "Alpha 的事"),
                ("mode", "full_text"),
            ],
            "项目/Alpha/02-日志/2026-09-10.md:3\t",
        ),
    ] {
        let response = f
            .get(route, Some(&session))
            .query(&params)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{route}");
        assert_headers(&response, true);
        assert_eq!(header(&response, "studiolo-instance-id"), instance);
        let text = response.text().await.unwrap();
        assert!(text.contains(expected), "{route}: {text}");
        assert!(!text.contains("秘密内容") && !text.contains("Beta"));
    }
    let response = f
        .get("/read/handshake", Some(&session))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(header(&response, "studiolo-session-id"), session);
    assert_eq!(header(&response, "studiolo-instance-id"), instance);
    assert_headers(&response, true);
    assert!(count_tokens(&response.text().await.unwrap()) <= HARD_BUDGET_TOKENS);

    // 读取返回后权威账本与派生索引均已存在，且实例保持一致。
    let reads = f
        .ledger()
        .query(&AuditFilter::default().with_action("content.read"))
        .unwrap();
    assert!(reads.iter().all(|record| {
        let payload = record.event().payload().as_value();
        payload["agent_type"] == "claude" && payload["agent_instance"] == instance
    }));
    let indexed = f
        .store()
        .query_reads(&ReadQuery {
            agent_type: Some("claude".into()),
            instance_id: Some(instance.clone()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    for tool in ["read_note", "list_entries", "search"] {
        assert!(indexed.iter().any(|read| read.tool == tool));
    }
    assert!(indexed.iter().all(|read| {
        reads
            .iter()
            .any(|event| event.offset() == read.ledger_offset)
    }));
    let served = f
        .ledger()
        .query(&AuditFilter::default().with_action("handshake.served"))
        .unwrap();
    assert_eq!(served.len(), 2);
    assert_eq!(
        served[0].event().payload().as_value()["token_count"],
        json!(count_tokens(&signpost))
    );
    assert!(
        served
            .iter()
            .all(|record| record.event().payload().as_value()["agent_instance"] == instance)
    );
    assert_eq!(
        f.ledger()
            .query(&AuditFilter::default().with_action("agent_instance.registered"))
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn each_new_handshake_creates_a_distinct_session_and_auditable_instance() {
    let (_temp, f) = fixture().await;
    let (first_session, first_instance, _) = f.handshake().await;
    let (second_session, second_instance, _) = f.handshake().await;
    assert_ne!(first_session, second_session);
    assert_ne!(first_instance, second_instance);
    assert_ne!(
        first_session, first_instance,
        "instance id must not be the session credential"
    );
}
