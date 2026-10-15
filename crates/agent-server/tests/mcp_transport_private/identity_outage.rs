// 身份库读不出来时（BUG-030）门禁的复核、响应流和供给复核怎么应答：结论未知，回 503、暂停交付、
// 留着会话，下一次再判；不能当成凭据吊销回 401，也不能当成会话失效回 404 或丢会话。
// 认证门之后的故障靠自造的层在指定时刻把身份库弄坏，才能确定地走到门禁自己的复核。
//
// 注入方式：私有库文件改成 0640。AgentStore 每次存取前先复核私有存储，复核不过即返回 Err，
// 与 SQLite 忙超过 busy_timeout、I/O 错走同一条 Err 路径；改回 0600 即恢复。

fn set_identity_store_mode(data: &Path, mode: u32) {
    fs::set_permissions(data.join("agent.sqlite3"), fs::Permissions::from_mode(mode)).unwrap();
}

/// 按真实握手的形状直接登记一个已初始化的会话：凭据摘要与认证门算出的一致。
fn live_binding(manager: &Manager, token: &str, session: &str) -> Arc<super::Binding> {
    let digest = <sha2::Sha256 as sha2::Digest>::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let binding = manager
        .pool
        .reserve("claude", &digest, Instant::now())
        .unwrap();
    binding.session.set(Arc::from(session)).unwrap();
    assert!(binding.initialize(Instant::now()));
    binding
}

/// 身份库在哪一刻坏掉：认证门放行之后、门禁查会话之前，或请求已交给内层之后。
#[derive(Clone, Copy)]
enum Breaks {
    BeforeSessionCheck,
    AfterDispatch,
}

struct Outcome {
    status: reqwest::StatusCode,
    challenge: Option<String>,
    body: String,
    session_kept: bool,
    reasons: Vec<String>,
}

async fn identity_store_breaks(at: Breaks) -> Outcome {
    let manager = bare_manager();
    let shared = manager.clone();
    // 夹具建好之后才知道私有目录在哪；层里的闭包经它拿路径。
    let data = Arc::new(std::sync::OnceLock::<std::path::PathBuf>::new());
    let (before, after) = (data.clone(), data.clone());
    let f = fixture_with_router(move |app| {
        let state = Arc::new(super::TransportState {
            app: app.clone(),
            manager: shared,
            requests: Arc::new(tokio::sync::Semaphore::new(16)),
        });
        axum::Router::new()
            .route(
                "/mcp",
                axum::routing::any(move || {
                    let data = after.clone();
                    async move {
                        if matches!(at, Breaks::AfterDispatch) {
                            set_identity_store_mode(data.get().unwrap(), 0o640);
                        }
                        "内层已处理"
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state,
                super::gate::gate,
            ))
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let data = before.clone();
                    async move {
                        if matches!(at, Breaks::BeforeSessionCheck) {
                            set_identity_store_mode(data.get().unwrap(), 0o640);
                        }
                        next.run(request).await
                    }
                },
            ))
            .layer(axum::middleware::from_fn_with_state(
                app,
                crate::auth::require_bearer,
            ))
    })
    .await;
    data.set(f.data.clone()).unwrap();
    let session = "01T180IDENTITYSTORE0000000";
    let binding = live_binding(&manager, &f.token, session);
    let response = f
        .request(
            Some(&f.token),
            Some(session),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        )
        .send()
        .await
        .unwrap();
    let status = response.status();
    let challenge = response
        .headers()
        .get("www-authenticate")
        .map(|value| value.to_str().unwrap().to_owned());
    let body = response.text().await.unwrap();
    set_identity_store_mode(&f.data, 0o600);
    let reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| event.event().payload().as_value()["reason"].as_str().unwrap().to_owned())
        .collect();
    Outcome {
        status,
        challenge,
        body,
        session_kept: manager.pool.find(session).is_some() && binding.live(Instant::now()),
        reasons,
    }
}

#[tokio::test]
async fn an_identity_store_failure_at_the_session_check_answers_503_not_404() {
    // 会话是否还有效未知：回 404 会让客户端丢掉可能仍有效的会话去重新握手。
    let outcome = identity_store_breaks(Breaks::BeforeSessionCheck).await;
    assert_eq!(outcome.status, 503);
    assert!(outcome.challenge.is_none());
    assert!(!outcome.body.contains("内层已处理"));
    assert!(outcome.session_kept);
    assert_eq!(outcome.reasons, ["identity_unavailable"]);
}

#[tokio::test]
async fn an_identity_store_failure_after_dispatch_answers_503_not_401() {
    // 凭据真假未知，不能当成途中被吊销回 401；请求可能已在执行，回 404 又会招来自动重放。
    let outcome = identity_store_breaks(Breaks::AfterDispatch).await;
    assert_eq!(outcome.status, 503);
    assert!(outcome.challenge.is_none());
    assert!(!outcome.body.contains("内层已处理"));
    assert!(outcome.session_kept);
    assert_eq!(outcome.reasons, ["identity_unavailable"]);
}

/// 内层响应体：测试端逐块推送，模拟 SDK 陆续产生的 SSE 事件。
struct Pushed(tokio::sync::mpsc::Receiver<Result<axum::body::Bytes, std::io::Error>>);

impl futures_core::Stream for Pushed {
    type Item = Result<axum::body::Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(context)
    }
}

/// 读下一块；流结束返回 `None`。
async fn next_chunk(stream: &mut axum::body::BodyDataStream) -> Option<axum::body::Bytes> {
    std::future::poll_fn(|context| {
        futures_core::Stream::poll_next(std::pin::Pin::new(&mut *stream), context)
    })
    .await
    .map(Result::unwrap)
}

#[tokio::test]
async fn an_unreadable_identity_store_pauses_a_stream_without_ending_it_or_the_session() {
    let f = fixture().await;
    let manager = bare_manager();
    let state = Arc::new(super::TransportState {
        app: f.state.clone(),
        manager: manager.clone(),
        requests: Arc::new(tokio::sync::Semaphore::new(1)),
    });
    let session = "01T180STREAMSESSION0000000";
    let binding = live_binding(&manager, &f.token, session);
    let permit = state.requests.clone().try_acquire_owned().unwrap();
    let (sender, receiver) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(4);
    let mut stream = super::body::GuardedBody::wrap(
        axum::body::Body::from_stream(Pushed(receiver)),
        state,
        binding.clone(),
        permit,
        None,
        true,
    )
    .into_data_stream();
    sender.send(Ok("第一块".into())).await.unwrap();
    assert_eq!(
        next_chunk(&mut stream).await.as_deref(),
        Some("第一块".as_bytes())
    );
    set_identity_store_mode(&f.data, 0o640);
    sender.send(Ok("第二块".into())).await.unwrap();
    // 读第二块的这个 future 从头挂到尾：身份库坏的 2.5 秒里不交付（凭据可能已被吊销），也不结束流；
    // 另一个任务在这期间确认会话没被丢，再把身份库修好。之后测试不再手动 poll，只有每秒的定时唤醒
    // 能叫醒它，这样才证明暂停时确实登记了唤醒，而不是一直挂死。
    let restorer = {
        let (data, manager, binding) = (f.data.clone(), manager.clone(), binding.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            let kept = manager.pool.find(session).is_some() && binding.live(Instant::now());
            set_identity_store_mode(&data, 0o600);
            kept
        })
    };
    let started = Instant::now();
    let resumed = tokio::time::timeout(Duration::from_secs(6), next_chunk(&mut stream)).await;
    let elapsed = started.elapsed();
    assert!(restorer.await.unwrap(), "身份库坏的这段时间会话不能被丢");
    assert!(elapsed >= Duration::from_millis(2500), "身份库坏时不能交付：{elapsed:?}");
    let resumed = resumed.expect("身份库修好后应由定时唤醒接着交付，而不是一直挂着");
    assert_eq!(resumed.as_deref(), Some("第二块".as_bytes()));
}

#[tokio::test]
async fn tool_calls_report_an_unreadable_identity_store_as_unavailable_not_as_a_dead_session() {
    let f = fixture().await;
    let manager = bare_manager();
    let binding = live_binding(&manager, &f.token, "01T180TOOLSESSION000000000");
    let session = crate::server::AgentSession::new_mcp(f.state.clone(), binding);
    set_identity_store_mode(&f.data, 0o640);
    let error = session.text_handshake().await.unwrap_err();
    set_identity_store_mode(&f.data, 0o600);
    // 「会话无效，请重新握手」会让客户端丢掉仍有效的会话；如实报身份库不可用，下一次调用再判。
    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{}",
        error.message
    );
    assert!(error.message.contains("身份库不可用"), "{}", error.message);
}

/// 接了策略平面的服务状态：只有这时 initialize 才会去查子实例的父链。
fn policy_state() -> (TempDir, std::path::PathBuf, Arc<AppState>) {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(&vault.join("项目/Alpha/00-总览/INDEX.md"), "# Alpha · 总览\n");
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));
    drop(AgentStore::open(&data).unwrap());
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());
    let policy = temp.path().join("policy-data");
    private_dir(&policy);
    drop(studiolo_policy::PolicyStore::open(&policy).unwrap());
    let state = serve::prepare(&ServeOptions {
        board_socket: None,
        index_cache: None,
        vault: Some(vault),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into()],
        data: data.clone(),
        policy: Some(policy),
        port: 0,
    })
    .unwrap();
    (temp, data, state)
}

#[tokio::test]
async fn a_parent_lookup_failure_is_reported_as_unavailable_not_as_an_invalid_parent() {
    let (_temp, data, state) = policy_state();
    let parent = "01T180PARENTINSTANCE000000";
    // 不变行为：身份库好好的、父实例确实不存在，仍是「父实例无效」。
    let missing = crate::server::AgentSession::new(state.clone())
        .initialize_with_parent("claude", Some(parent))
        .await
        .unwrap_err();
    assert_eq!(missing.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(missing.message.contains("父实例无效"), "{}", missing.message);
    set_identity_store_mode(&data, 0o640);
    let error = crate::server::AgentSession::new(state)
        .initialize_with_parent("claude", Some(parent))
        .await
        .unwrap_err();
    set_identity_store_mode(&data, 0o600);
    assert_eq!(
        error.code,
        rmcp::model::ErrorCode::INTERNAL_ERROR,
        "{}",
        error.message
    );
    assert!(error.message.contains("身份库不可用"), "{}", error.message);
}
