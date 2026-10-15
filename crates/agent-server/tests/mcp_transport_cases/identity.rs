#[tokio::test]
async fn another_authenticated_type_cannot_post_listen_replay_notify_or_delete_a_session() {
    let f = fixture().await;
    let other = f.store().issue_type("codex", "2026-09-12T00:00:00Z").unwrap().token;
    let (session, _) = f.initialize(&f.token).await;
    // 别的主体拿来的会话号与查不到的一样回 404、不带 Bearer challenge：不暴露会话是否存在，
    // 也不让对方的客户端误以为自己的令牌失效（BUG-029）。
    for method in [reqwest::Method::POST, reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = transport(&f, method, &other, &session)
            .body(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"read_note","arguments":{"path":"项目/Alpha/00-总览/INDEX.md"}
            }}).to_string()).send().await.unwrap();
        assert_eq!(response.status(), 404);
        assert!(response.headers().get("www-authenticate").is_none());
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = response.text().await.unwrap();
        assert!(!body.contains("Alpha") && !body.contains(&session) && !body.contains(&other));
    }
    let replay = transport(&f, reqwest::Method::GET, &other, &session)
        .header("Last-Event-ID", "0/0").send().await.unwrap();
    assert_eq!(replay.status(), 404);
    let notification = f.request(Some(&other), Some(&session), json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send().await.unwrap();
    assert_eq!(notification.status(), 404);
    let denied = f.ledger().query(&AuditFilter::default().with_action("agent.action_denied")).unwrap();
    assert_eq!(denied.len(), 5);
    for event in denied {
        let payload = event.event().payload().as_value();
        assert_eq!(payload["agent_type"], "codex");
        assert!(payload["agent_instance"].is_null());
        assert_eq!(payload["reason"], "session_invalid");
    }
    // 错身份的 DELETE 没有杀死原会话；原主体及另一个独立主体均可继续正常读取。
    let read = f.call(&f.token, &session, 3, "read_note", json!({"path":"项目/Alpha/00-总览/INDEX.md"})).await;
    assert!(!is_error(&read));
    assert!(tool_text(&read).contains("Alpha · 总览"));
    let (other_session, _) = f.initialize(&other).await;
    assert_ne!(session, other_session);
    assert!(!is_error(&f.call(&other, &other_session, 4, "handshake", json!({})).await));
}
