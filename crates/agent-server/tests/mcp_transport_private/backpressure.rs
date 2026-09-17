// SDK 在队列发送中不监听取消；两边背压都必须通过关闭或排空解除。
async fn manual_transport() -> (
    Arc<Manager>,
    Arc<super::Binding>,
    rmcp::transport::streamable_http_server::session::SessionId,
    super::worker::Worker,
) {
    use rmcp::transport::Transport;
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let (id, mut transport) = REQUEST_BINDING
        .scope(binding.clone(), manager.create_session())
        .await
        .unwrap();
    let initialize =
        manager.initialize_session(&id, serde_json::from_value(initialization()).unwrap());
    let handler = async {
        assert!(transport.receive().await.is_some());
        transport.send(serde_json::from_value(json!({"jsonrpc":"2.0","id":1,"result":{
            "protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"test","version":"0"}
        }})).unwrap()).await.unwrap();
    };
    let (response, ()) = tokio::join!(initialize, handler);
    assert!(response.is_ok());
    (manager, binding, id, transport)
}

fn unrouted_reply(id: i64) -> rmcp::model::ServerJsonRpcMessage {
    serde_json::from_value(json!({"jsonrpc":"2.0","id":id,
        "error":{"code":-32603,"message":"无内容背压夹具"}}))
    .unwrap()
}

#[tokio::test]
async fn unpolled_full_sse_is_dropped_before_waiting_for_worker_join() {
    use futures_core::Stream;
    use rmcp::transport::Transport;
    let (manager, binding, id, mut transport) = manual_transport().await;
    let mut stream = Box::pin(manager.create_standalone_stream(&id).await.unwrap());
    for id in 2..18 {
        transport.send(unrouted_reply(id)).await.unwrap();
    }
    let blocked = tokio::time::timeout(
        Duration::from_millis(20),
        transport.send(unrouted_reply(18)),
    )
    .await;
    assert!(blocked.is_err(), "未消费的 16 帧 SDK 队列应产生真实背压");
    manager.discard(binding);
    empty(&manager).await;
    assert!(
        std::future::poll_fn(|context| stream.as_mut().poll_next(context))
            .await
            .is_none()
    );
    transport.close().await.unwrap();
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn full_client_queue_is_drained_during_close_without_a_handler_consumer() {
    use rmcp::transport::Transport;
    let (manager, binding, id, mut transport) = manual_transport().await;
    let notification: rmcp::model::ClientJsonRpcMessage =
        serde_json::from_value(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .unwrap();
    let blocked = tokio::time::timeout(Duration::from_millis(20), async {
        for _ in 0..128 {
            manager
                .accept_message(&id, notification.clone())
                .await
                .unwrap();
        }
    })
    .await;
    assert!(blocked.is_err(), "SDK 与代理的有界接收队列应填满");
    manager.discard(binding);
    empty(&manager).await;
    assert!(transport.receive().await.is_none());
    transport.close().await.unwrap();
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}
