/// 内层收到请求后做的事：什么都不做、让会话结束、吊销类型令牌、两件同时发生。
#[derive(Clone, Copy)]
enum AfterDispatch {
    Nothing,
    EndSession,
    RevokeToken,
    RevokeTokenAndEndSession,
}

/// 门禁在请求交给内层之后还要复核一次会话。用自造的内层代替 rmcp，才能确定地让会话在交出之后、
/// 返回之前失效；会话号与凭据摘要按真实握手的形状直接登记进池子。返回（状态码、challenge、正文）。
async fn dispatch_then(
    method: reqwest::Method,
    change: AfterDispatch,
) -> (reqwest::StatusCode, Option<String>, String) {
    let manager = bare_manager();
    let shared = manager.clone();
    let f = fixture_with_router(move |app| {
        let state = Arc::new(super::TransportState {
            app: app.clone(),
            manager: shared,
            requests: Arc::new(tokio::sync::Semaphore::new(16)),
        });
        let inner = app.clone();
        axum::Router::new()
            .route(
                "/mcp",
                axum::routing::any(move || {
                    let app = inner.clone();
                    async move {
                        let binding = REQUEST_BINDING.with(Arc::clone);
                        let revoke = || {
                            assert!(
                                app.store
                                    .revoke_type("claude", "2026-09-12T00:00:01Z")
                                    .unwrap()
                            );
                        };
                        match change {
                            AfterDispatch::Nothing => {}
                            AfterDispatch::EndSession => binding.stop_worker(),
                            AfterDispatch::RevokeToken => revoke(),
                            AfterDispatch::RevokeTokenAndEndSession => {
                                revoke();
                                binding.stop_worker();
                            }
                        }
                        "内层已处理"
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state,
                super::gate::gate,
            ))
            .layer(axum::middleware::from_fn_with_state(
                app,
                crate::auth::require_bearer,
            ))
    })
    .await;
    let digest = <sha2::Sha256 as sha2::Digest>::digest(f.token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let binding = manager
        .pool
        .reserve("claude", &digest, Instant::now())
        .unwrap();
    let session = "01T179DISPATCHEDSESSION000";
    binding.session.set(Arc::from(session)).unwrap();
    assert!(binding.initialize(Instant::now()));
    let response = f
        .client
        .request(method, &f.base)
        .bearer_auth(&f.token)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("MCP-Protocol-Version", "2025-11-25")
        .header("Mcp-Session-Id", session)
        .body(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}).to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let challenge = response
        .headers()
        .get("www-authenticate")
        .map(|value| value.to_str().unwrap().to_owned());
    (status, challenge, response.text().await.unwrap())
}

#[tokio::test]
async fn a_session_still_valid_after_dispatch_passes_the_response_through() {
    let (status, challenge, body) =
        dispatch_then(reqwest::Method::POST, AfterDispatch::Nothing).await;
    assert_eq!(status, 200);
    assert!(challenge.is_none());
    assert_eq!(body, "内层已处理");
}

#[tokio::test]
async fn a_session_ended_after_dispatch_answers_503_not_404_or_401() {
    // 请求可能已在执行、结果未知：回 404 会让客户端当成「没处理过」而重新握手后自动重放，
    // 回 401 会让客户端当成令牌失效（BUG-029）。
    let (status, challenge, body) =
        dispatch_then(reqwest::Method::POST, AfterDispatch::EndSession).await;
    assert_eq!(status, 503);
    assert!(challenge.is_none());
    assert!(!body.contains("内层已处理"));
}

#[tokio::test]
async fn a_token_revoked_after_dispatch_answers_401_with_a_challenge() {
    let (status, challenge, body) =
        dispatch_then(reqwest::Method::POST, AfterDispatch::RevokeToken).await;
    assert_eq!(status, 401);
    assert_eq!(
        challenge.as_deref(),
        Some("Bearer realm=\"studiolo-agent\"")
    );
    assert!(!body.contains("内层已处理"));
}

#[tokio::test]
async fn a_revoked_token_wins_over_an_ended_session_after_dispatch() {
    // 吊销之后后台每秒的清理也会把会话关掉；先判凭据，客户端才稳定拿到该换令牌的 401，而不是 503。
    let (status, challenge, body) = dispatch_then(
        reqwest::Method::POST,
        AfterDispatch::RevokeTokenAndEndSession,
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(
        challenge.as_deref(),
        Some("Bearer realm=\"studiolo-agent\"")
    );
    assert!(!body.contains("内层已处理"));
}

#[tokio::test]
async fn delete_skips_the_post_dispatch_check_because_it_closes_the_session_itself() {
    let (status, challenge, body) =
        dispatch_then(reqwest::Method::DELETE, AfterDispatch::EndSession).await;
    assert_eq!(status, 200);
    assert!(challenge.is_none());
    assert_eq!(body, "内层已处理");
}
