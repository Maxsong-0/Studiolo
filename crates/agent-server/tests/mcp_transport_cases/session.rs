// 会话失效时的状态码（BUG-029）：按 MCP 2025-11-25 会话管理，会话不可用回 404、缺会话号回 400；
// 401 与 Bearer challenge 只留给凭据本身无效，否则客户端会把过期会话当成令牌失效、转去重新认证。

#[tokio::test]
async fn unknown_session_ids_answer_404_without_a_challenge_and_a_new_initialize_recovers() {
    let f = fixture().await;
    // 形状合法、从未发出过的会话号：与闲置或到期后被回收的会话走同一条路。
    let unknown = "01T179UNKNOWNSESSION000000";
    for method in [reqwest::Method::POST, reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = transport(&f, method.clone(), &f.token, unknown)
            .body(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{method}");
        assert!(response.headers().get("www-authenticate").is_none(), "{method}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(!response.text().await.unwrap().contains(unknown));
    }
    let notification = f
        .request(Some(&f.token), Some(unknown), json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(notification.status(), 404);
    assert!(notification.headers().get("www-authenticate").is_none());
    let denied = f.ledger().query(&AuditFilter::default().with_action("agent.action_denied")).unwrap();
    assert_eq!(denied.len(), 4);
    for event in denied {
        let payload = event.event().payload().as_value();
        assert_eq!(payload["agent_type"], "claude");
        assert_eq!(payload["reason"], "session_invalid");
    }
    // 客户端按规范不带会话号重新 initialize，同一令牌立即可用。
    let (session, _) = f.initialize(&f.token).await;
    assert_ne!(session, unknown);
    assert!(!is_error(&f.call(&f.token, &session, 3, "handshake", json!({})).await));
}

#[tokio::test]
async fn missing_or_wrong_tokens_answer_401_with_a_challenge_even_with_a_live_session_id() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    for token in [None, Some("sag1_wrong")] {
        let response = f
            .request(token, Some(&session), json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["www-authenticate"], "Bearer realm=\"studiolo-agent\"");
        assert!(response.text().await.unwrap().is_empty());
    }
    // 认证门在会话判断之前拦下，不碰会话：原会话照常可用。
    assert!(!is_error(&f.call(&f.token, &session, 3, "handshake", json!({})).await));
}

#[tokio::test]
async fn requests_without_a_session_id_other_than_initialize_answer_400() {
    let f = fixture().await;
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = f
            .client
            .request(method.clone(), &f.base)
            .bearer_auth(&f.token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "{method}");
        assert!(response.headers().get("www-authenticate").is_none(), "{method}");
    }
    let listed = f
        .request(Some(&f.token), None, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), 400);
    assert!(listed.headers().get("www-authenticate").is_none());
    let mut reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| event.event().payload().as_value()["reason"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    reasons.sort();
    assert_eq!(reasons, ["initialize_invalid", "session_required", "session_required"]);
}
