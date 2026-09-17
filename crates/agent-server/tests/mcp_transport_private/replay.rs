#[tokio::test]
async fn completed_posts_close_immediately_and_never_replay_old_results() {
    let (f, manager) = bounded(Limits::default()).await;
    assert_eq!(
        manager.inner.session_config.completed_cache_ttl,
        Duration::ZERO
    );
    let (session, _) = f.initialize(&f.token).await;
    for id in 2..22 {
        let response = f
            .request(
                Some(&f.token),
                Some(&session),
                json!({"jsonrpc":"2.0","id":id,"method":"tools/list"}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.text().await.unwrap();
        assert!(body.contains("read_note"));
        let last = body
            .lines()
            .filter_map(|line| line.strip_prefix("id:"))
            .next_back()
            .unwrap()
            .trim();
        let replay = f
            .client
            .get(&f.base)
            .bearer_auth(&f.token)
            .header("Accept", "text/event-stream")
            .header("Mcp-Session-Id", &session)
            .header("Last-Event-ID", last)
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), 200);
        assert!(
            replay.text().await.unwrap().is_empty(),
            "不存在的历史流不能冒充成功回放旧结果"
        );
    }
    assert_eq!(manager.pool.all().len(), 1);
    assert_eq!(manager.inner.sessions.read().await.len(), 1);
}

#[tokio::test]
async fn cancelled_request_caches_are_evicted_before_processing_the_next_event() {
    use futures_core::Stream;
    use rmcp::transport::Transport;
    let mut manager = LocalSessionManager::default();
    manager.session_config.completed_cache_ttl = Duration::ZERO;
    let (session, mut transport) = manager.create_session().await.unwrap();
    let worker = tokio::spawn(async move {
        assert!(transport.receive().await.is_some());
        let response = serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":{
            "protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"test","version":"0"}
        }})).unwrap();
        transport.send(response).await.unwrap();
        // 保留未完成请求，随后由 cancellation 分支关闭，避免只测正常完成的立即移除分支。
        while transport.receive().await.is_some() {}
        transport.close().await.unwrap();
    });
    manager
        .initialize_session(&session, serde_json::from_value(initialization()).unwrap())
        .await
        .unwrap();
    for id in 2..22 {
        let request =
            serde_json::from_value(json!({"jsonrpc":"2.0","id":id,"method":"tools/list"})).unwrap();
        let mut stream = Box::pin(manager.create_stream(&session, request).await.unwrap());
        let priming = std::future::poll_fn(|context| stream.as_mut().poll_next(context))
            .await
            .unwrap();
        let event = priming.event_id.unwrap();
        assert!(
            manager.resume(&session, event.clone()).await.is_ok(),
            "未完成活流仍支持重连"
        );
        let cancel =
            serde_json::from_value(json!({"jsonrpc":"2.0","method":"notifications/cancelled",
            "params":{"requestId":id,"reason":"test"}}))
            .unwrap();
        manager.accept_message(&session, cancel).await.unwrap();
        // 同一 worker 的事件 FIFO：先 cancellation，再 resume；ZERO 必须已删除 completed 通道。
        assert!(manager.resume(&session, event).await.is_err());
    }
    manager.close_session(&session).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
}
