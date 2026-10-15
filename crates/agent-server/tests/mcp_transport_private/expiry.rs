/// 闲置或绝对期限到点后，带旧会话号一律 404、不带 Bearer challenge，不带会话号重新 initialize
/// 即恢复（BUG-029）：Claude Code 收到 404 会自己重新握手，收到 401 则当成令牌失效转去重新认证。
async fn expired_session_answers_404_until_a_new_initialize(limits: Limits, wait: Duration) {
    let (f, _manager) = bounded(limits).await;
    let (session, _) = f.initialize(&f.token).await;
    assert!(!is_error(
        &f.call(&f.token, &session, 2, "handshake", json!({})).await
    ));
    tokio::time::sleep(wait).await;
    for method in [reqwest::Method::POST, reqwest::Method::GET] {
        let response = f
            .client
            .request(method.clone(), &f.base)
            .bearer_auth(&f.token)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Mcp-Session-Id", &session)
            .body(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {}}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{method}");
        assert!(
            response.headers().get("www-authenticate").is_none(),
            "{method}"
        );
    }
    let (renewed, _) = f.initialize(&f.token).await;
    assert_ne!(renewed, session);
    assert!(!is_error(
        &f.call(&f.token, &renewed, 4, "handshake", json!({})).await
    ));
}

#[tokio::test]
async fn idle_expiry_answers_404_and_a_new_initialize_recovers() {
    expired_session_answers_404_until_a_new_initialize(
        Limits {
            idle: Duration::from_secs(1),
            ..Limits::default()
        },
        Duration::from_millis(1500),
    )
    .await;
}

#[tokio::test]
async fn absolute_expiry_answers_404_and_a_new_initialize_recovers() {
    expired_session_answers_404_until_a_new_initialize(
        Limits {
            absolute: Duration::from_secs(2),
            ..Limits::default()
        },
        Duration::from_millis(2500),
    )
    .await;
}
