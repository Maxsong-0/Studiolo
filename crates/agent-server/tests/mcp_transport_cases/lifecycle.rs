#[tokio::test]
async fn revoked_tokens_stop_every_transport_including_an_already_open_sse() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    let mut stream = transport(&f, reqwest::Method::GET, &f.token, &session).send().await.unwrap();
    assert_eq!(stream.status(), 200);
    f.store().revoke_type("claude", "2026-09-12T00:00:01Z").unwrap();
    for method in [reqwest::Method::POST, reqwest::Method::GET, reqwest::Method::DELETE] {
        assert_eq!(transport(&f, method, &f.token, &session).send().await.unwrap().status(), 401);
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            assert!(!String::from_utf8_lossy(&chunk).contains("Alpha"));
        }
    }).await.expect("已建立 SSE 必须随撤权结束");
}

#[tokio::test]
async fn rotating_a_type_token_invalidates_old_stream_and_session_even_after_reissue() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    let mut stream = transport(&f, reqwest::Method::GET, &f.token, &session).send().await.unwrap();
    assert_eq!(stream.status(), 200);
    let new = f.store().issue_type("claude", "2026-09-12T00:00:01Z").unwrap().token;
    // 旧令牌在认证门回 401；新令牌带旧会话号按查不到处理回 404，客户端据此重新 initialize。
    assert_eq!(transport(&f, reqwest::Method::GET, &f.token, &session).send().await.unwrap().status(), 401);
    assert_eq!(transport(&f, reqwest::Method::GET, &new, &session).send().await.unwrap().status(), 404);
    tokio::time::timeout(Duration::from_secs(3), async {
        while stream.chunk().await.unwrap().is_some() {}
    }).await.unwrap();
    let (new_session, _) = f.initialize(&new).await;
    assert_ne!(session, new_session);
}
