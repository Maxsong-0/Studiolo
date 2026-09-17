// 使用真实 rmcp handler 延迟应答，确认传输回收不等同于实际供给 future 完成。
struct DelayedInitialize {
    binding: Arc<super::Binding>,
    entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Arc<tokio::sync::Semaphore>,
}

impl rmcp::ServerHandler for DelayedInitialize {
    async fn initialize(
        &self,
        _: rmcp::model::InitializeRequestParams,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
        let _work = self.binding.enter_work().unwrap();
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        let _released = self.release.acquire().await.unwrap();
        Ok(self.get_info())
    }
}

#[tokio::test]
async fn late_real_handler_keeps_work_permit_and_cannot_replace_the_closed_handshake() {
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let (id, transport) = REQUEST_BINDING
        .scope(binding.clone(), manager.create_session())
        .await
        .unwrap();
    let held = (0..15)
        .map(|_| binding.enter_work().unwrap())
        .collect::<Vec<_>>();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let handler = DelayedInitialize {
        binding: binding.clone(),
        entered: std::sync::Mutex::new(Some(entered)),
        release: release.clone(),
    };
    let serving = tokio::spawn(async move { rmcp::serve_server(handler, transport).await });
    let task_manager = manager.clone();
    let initializing = tokio::spawn(async move {
        task_manager
            .initialize_session(&id, serde_json::from_value(initialization()).unwrap())
            .await
    });
    ready.await.unwrap();
    manager.discard(binding.clone());
    let result = tokio::time::timeout(Duration::from_secs(3), initializing)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    empty(&manager).await;
    assert!(
        !serving.is_finished(),
        "不能通过取消真实 handler 冒充其工作已结束"
    );
    let replacement = manager
        .pool
        .reserve("codex", "digest", Instant::now())
        .unwrap();
    assert!(
        replacement.enter_work().is_none(),
        "实际供给许可必须仍由迟到 handler 持有"
    );
    release.add_permits(1);
    let late = tokio::time::timeout(Duration::from_secs(3), serving)
        .await
        .unwrap()
        .unwrap();
    assert!(late.is_err(), "迟到成功应答不能覆盖已关闭的握手");
    assert!(replacement.enter_work().is_some());
    assert!(!binding.live(Instant::now()));
    assert!(!replacement.live(Instant::now()));
    drop(held);
}

#[tokio::test]
async fn buffered_initialize_is_not_delivered_to_a_handler_after_worker_cleanup() {
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
    let result = tokio::time::timeout(
        Duration::from_millis(20),
        manager.initialize_session(&id, serde_json::from_value(initialization()).unwrap()),
    )
    .await;
    assert!(result.is_err());
    manager.discard(binding);
    empty(&manager).await;
    assert!(
        transport.receive().await.is_none(),
        "关闭后不能交付代理中排队的旧初始化"
    );
    transport.close().await.unwrap();
}

#[tokio::test]
async fn confirmed_real_initialize_is_never_replaced_by_a_cleanup_reply() {
    struct Ready;
    impl rmcp::ServerHandler for Ready {}
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let (id, transport) = REQUEST_BINDING
        .scope(binding.clone(), manager.create_session())
        .await
        .unwrap();
    let serving = tokio::spawn(async move { rmcp::serve_server(Ready, transport).await });
    let response = manager
        .initialize_session(&id, serde_json::from_value(initialization()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        response,
        rmcp::model::ServerJsonRpcMessage::Response(_)
    ));
    assert!(binding.live(Instant::now()));
    let service = serving.await.unwrap().unwrap();
    manager.discard(binding.clone());
    assert!(
        binding.initialize_failure().is_none(),
        "已确认的真实应答不能再补失败应答"
    );
    empty(&manager).await;
    tokio::time::timeout(Duration::from_secs(3), service.waiting())
        .await
        .unwrap()
        .unwrap();
}
