#[tokio::test]
async fn expiry_closes_an_open_sse_stream_and_restores_capacity_without_new_requests() {
    let (f, manager) = bounded(Limits {
        total: 1,
        idle: Duration::from_millis(500),
        absolute: Duration::from_secs(2),
        ..Limits::default()
    })
    .await;
    let (session, _) = f.initialize(&f.token).await;
    let mut stream = f
        .client
        .get(&f.base)
        .bearer_auth(&f.token)
        .header("Accept", "text/event-stream")
        .header("Mcp-Session-Id", session)
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);
    tokio::time::timeout(Duration::from_secs(3), async {
        while stream.chunk().await.unwrap().is_some() {}
    })
    .await
    .expect("已打开 SSE 到期后必须结束");
    empty(&manager).await;
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn open_sse_connections_hold_request_capacity_until_their_bodies_are_dropped() {
    let (f, _) = bounded(Limits::default()).await;
    let (session, _) = f.initialize(&f.token).await;
    let mut streams = Vec::new();
    for _ in 0..16 {
        let response = f
            .client
            .get(&f.base)
            .bearer_auth(&f.token)
            .header("Accept", "text/event-stream")
            .header("Mcp-Session-Id", &session)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        streams.push(response);
    }
    let busy = f
        .request(
            Some(&f.token),
            Some(&session),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), 429);
    drop(streams);
    // 对端断开被服务端观察后，许可恢复；不靠会话过期释放。
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let response = f
                .request(
                    Some(&f.token),
                    Some(&session),
                    json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
                )
                .send()
                .await
                .unwrap();
            if response.status() == 200 {
                break;
            }
            assert_eq!(response.status(), 429);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
