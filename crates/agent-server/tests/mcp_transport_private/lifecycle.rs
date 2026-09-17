fn bare_manager() -> Arc<Manager> {
    Arc::new(Manager {
        inner: Arc::new(LocalSessionManager::default()),
        pool: Arc::new(Pool::new(Limits {
            total: 1,
            ..Limits::default()
        })),
    })
}

#[tokio::test]
async fn cleanup_waits_for_a_creation_blocked_on_the_sdk_lock_to_cancel() {
    use std::{future::Future, task::Poll};
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let lock = manager.inner.sessions.write().await;
    {
        let creating = REQUEST_BINDING.scope(binding.clone(), manager.create_session());
        tokio::pin!(creating);
        std::future::poll_fn(|context| {
            assert!(creating.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        manager.discard(binding);
        tokio::task::yield_now().await;
        assert_eq!(manager.pool.all().len(), 1);
        assert!(
            manager
                .pool
                .reserve("codex", "digest", Instant::now())
                .is_err()
        );
        // 退出作用域取消仍未拿到 SDK 锁的创建；Creation 的 Drop 会唤醒清理。
    }
    drop(lock);
    empty(&manager).await;
}

#[tokio::test]
async fn worker_join_does_not_release_capacity_before_sdk_metadata_cleanup() {
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let (_, transport) = REQUEST_BINDING
        .scope(binding.clone(), manager.create_session())
        .await
        .unwrap();
    let lock = manager.inner.sessions.write().await;
    manager.discard(binding.clone());
    tokio::time::timeout(Duration::from_secs(3), binding.wait_worker())
        .await
        .unwrap();
    assert_eq!(lock.len(), 1);
    assert_eq!(
        manager.pool.all().len(),
        1,
        "实际 worker 已退出，SDK 元数据仍占用容量"
    );
    assert!(
        manager
            .pool
            .reserve("codex", "digest", Instant::now())
            .is_err()
    );
    drop(lock);
    empty(&manager).await;
    drop(transport);
    assert!(
        manager
            .pool
            .reserve("codex", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn stalled_initialization_timeout_can_cleanup_worker_and_restore_admission() {
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let reservation = Reservation::new(binding.clone(), manager.clone());
    let (id, transport) = REQUEST_BINDING
        .scope(binding, manager.create_session())
        .await
        .unwrap();
    let message = serde_json::from_value(initialization()).unwrap();
    let response = tokio::time::timeout(
        Duration::from_millis(20),
        manager.initialize_session(&id, message),
    )
    .await;
    assert!(response.is_err(), "没有服务接收者时初始化不会完成");
    drop(reservation);
    empty(&manager).await;
    drop(transport);
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn closing_wakes_pending_initialize_without_dropping_its_caller_or_transport() {
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
    let task_manager = manager.clone();
    let initializing = tokio::spawn(async move {
        task_manager
            .initialize_session(&id, serde_json::from_value(initialization()).unwrap())
            .await
    });
    assert!(transport.receive().await.is_some());
    manager.discard(binding.clone());
    let result = tokio::time::timeout(Duration::from_secs(3), initializing)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err(), "关闭协议应答不能发布为成功握手");
    empty(&manager).await;
    assert!(!binding.live(Instant::now()));
    transport.close().await.unwrap();
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn cancellation_after_actual_session_creation_closes_worker_and_reservation() {
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let reservation = Reservation::new(binding.clone(), manager.clone());
    let task_manager = manager.clone();
    let (sent, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(REQUEST_BINDING.scope(binding, async move {
        let _reservation = reservation;
        let (_id, _transport) = task_manager.create_session().await.unwrap();
        sent.send(()).unwrap();
        std::future::pending::<()>().await;
    }));
    received.await.unwrap();
    assert_eq!(manager.inner.sessions.read().await.len(), 1);
    task.abort();
    let _ = task.await;
    empty(&manager).await;
    assert!(
        manager
            .pool
            .reserve("claude", "digest", Instant::now())
            .is_ok()
    );
}

#[tokio::test]
async fn failing_initialize_after_worker_creation_is_not_published_or_retained() {
    struct Reject;
    impl rmcp::ServerHandler for Reject {
        async fn initialize(
            &self,
            _: rmcp::model::InitializeRequestParams,
            _: rmcp::service::RequestContext<rmcp::RoleServer>,
        ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
            Err(rmcp::ErrorData::internal_error("初始化失败夹具", None))
        }
    }
    let manager = bare_manager();
    let binding = manager
        .pool
        .reserve("claude", "digest", Instant::now())
        .unwrap();
    let reservation = Reservation::new(binding.clone(), manager.clone());
    let (id, transport) = REQUEST_BINDING
        .scope(binding.clone(), manager.create_session())
        .await
        .unwrap();
    let worker = tokio::spawn(async move {
        let _ = rmcp::serve_server(Reject, transport).await;
    });
    let message = serde_json::from_value(initialization()).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        manager.initialize_session(&id, message),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!binding.live(Instant::now()));
    drop(reservation);
    empty(&manager).await;
    worker.await.unwrap();
}
