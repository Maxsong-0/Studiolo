// 关停（T-184，BUG-033）：开始关停后门禁对新会话、新请求、新 GET 流回 503 并记 server_stopping，
// DELETE 照常；已进门的请求做完之前不关会话，做完立即关全部会话，开着的 GET 流随之结束；
// 做不完的请求也只能拖到排空上限。

fn stopping_denials(f: &Fixture) -> usize {
    f.ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .filter(|event| event.event().payload().as_value()["reason"] == "server_stopping")
        .count()
}

async fn open_stream(f: &Fixture, session: &str) -> reqwest::Response {
    f.client
        .get(&f.base)
        .bearer_auth(&f.token)
        .header("Accept", "text/event-stream")
        .header("Mcp-Session-Id", session)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn stopping_refuses_new_work_and_closes_every_session_once_drained() {
    let (f, manager) = bounded(Limits::default()).await;
    let (kept, _) = f.initialize(&f.token).await;
    let (deleted, _) = f.initialize(&f.token).await;
    let mut stream = open_stream(&f, &kept).await;
    assert_eq!(stream.status(), 200);
    // 一个关停前已进门、还没做完的请求。
    let held = f.state.lifecycle.enter().expect("关停前可以进门");
    f.state.lifecycle.stop();

    let fresh = f
        .request(Some(&f.token), None, initialization())
        .send()
        .await
        .unwrap();
    assert_eq!(fresh.status(), 503, "关停后不接新会话");
    let call = f
        .request(
            Some(&f.token),
            Some(&kept),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(call.status(), 503, "关停后已有会话也不接新请求");
    assert_eq!(
        open_stream(&f, &kept).await.status(),
        503,
        "关停后不开新的 GET 流"
    );
    assert_eq!(stopping_denials(&f), 3);
    let closed = f
        .client
        .delete(&f.base)
        .bearer_auth(&f.token)
        .header("Mcp-Session-Id", &deleted)
        .send()
        .await
        .unwrap();
    assert!(closed.status().is_success(), "DELETE 照常关会话");

    // 排空之前不关别的会话，开着的 GET 流也还在。
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        manager
            .pool
            .find(&kept)
            .is_some_and(|binding| binding.live(Instant::now()))
    );
    drop(held);
    tokio::time::timeout(Duration::from_secs(3), async {
        while stream.chunk().await.unwrap().is_some() {}
    })
    .await
    .expect("排空后开着的 GET 流必须随会话关闭而结束");
    empty(&manager).await;
}

#[tokio::test]
async fn a_request_that_never_finishes_holds_sessions_open_only_until_the_drain_limit() {
    let (f, manager) = bounded(Limits::default()).await;
    let (session, _) = f.initialize(&f.token).await;
    let mut stream = open_stream(&f, &session).await;
    assert_eq!(stream.status(), 200);
    let _stuck = f.state.lifecycle.enter().expect("关停前可以进门");
    let limit = crate::shutdown::DRAIN_LIMIT;
    let started = Instant::now();
    f.state.lifecycle.stop();
    tokio::time::timeout(limit + Duration::from_secs(3), async {
        while stream.chunk().await.unwrap().is_some() {}
    })
    .await
    .expect("排空到上限后必须强制关会话，GET 流随之结束");
    let elapsed = started.elapsed();
    assert!(elapsed >= limit, "还没到排空上限就关了会话：{elapsed:?}");
    empty(&manager).await;
}
