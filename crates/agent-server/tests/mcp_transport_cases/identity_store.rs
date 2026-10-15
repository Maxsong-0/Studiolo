// 身份库读不出来时（BUG-030）令牌真假、会话是否有效都未知：认证门回 503、不带 Bearer challenge，
// 客户端稍后重试即可；回 401 会被 Claude Code 当成令牌失效，转去重新认证、卡在 needs_auth。
// 后台清理也不能把这段时间里的会话当成已吊销丢掉，恢复后同一会话照常可用。
//
// 注入方式：私有库文件改成 0640。AgentStore 每次存取前先复核私有存储，复核不过即返回 Err，
// 与 SQLite 忙超过 busy_timeout、I/O 错走同一条 Err 路径；改回 0600 即恢复。

fn set_identity_store_mode(f: &Fixture, mode: u32) {
    fs::set_permissions(f.data.join("agent.sqlite3"), fs::Permissions::from_mode(mode)).unwrap();
}

#[tokio::test]
async fn an_unreadable_identity_store_answers_503_without_a_challenge_and_keeps_the_session() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    set_identity_store_mode(&f, 0o640);
    let tools = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
    for method in [reqwest::Method::POST, reqwest::Method::GET] {
        let response = transport(&f, method.clone(), &f.token, &session)
            .body(tools.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 503, "{method}");
        assert!(response.headers().get("www-authenticate").is_none(), "{method}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = response.text().await.unwrap();
        assert!(
            !body.contains(&session) && !body.contains(&f.token) && !body.contains("agent.sqlite3"),
            "{body}"
        );
    }
    // 形状合法的令牌要查库才知道真假，查不了就一律 503：新握手、从未签发过的令牌都一样，也不铸实例。
    let unknown = format!("sag1_{}", "0".repeat(64));
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "contract", "version": "0"}
    }});
    for token in [f.token.as_str(), unknown.as_str()] {
        let response = f.request(Some(token), None, initialize.clone()).send().await.unwrap();
        assert_eq!(response.status(), 503);
        assert!(response.headers().get("www-authenticate").is_none());
    }
    // 不用查库就能判死的照旧 401 加 challenge：缺令牌、格式不对的令牌。
    for token in [None, Some("sag1_wrong")] {
        let response = f.request(token, Some(&session), tools.clone()).send().await.unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["www-authenticate"], "Bearer realm=\"studiolo-agent\"");
    }
    // 后台清理每秒一轮，让它至少跑两轮：读不出身份库不算凭据吊销，会话不能被丢掉。
    tokio::time::sleep(Duration::from_millis(2500)).await;
    set_identity_store_mode(&f, 0o600);
    let read = f
        .call(&f.token, &session, 3, "read_note", json!({"path": "项目/Alpha/00-总览/INDEX.md"}))
        .await;
    assert!(!is_error(&read));
    assert!(tool_text(&read).contains("Alpha · 总览"));
    // 每个 503 记 identity_unavailable（令牌还没认出来，主体匿名），与令牌无效的 unauthenticated 分开。
    let mut reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| event.event().payload().as_value()["reason"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            "identity_unavailable",
            "identity_unavailable",
            "identity_unavailable",
            "identity_unavailable",
            "unauthenticated",
            "unauthenticated",
        ]
    );
    // 两次回 503 的 initialize 都没铸实例：全程只有最初那次握手登记过一个。
    assert_eq!(f.ledger().query(&AuditFilter::default().with_action("agent_instance.registered")).unwrap().len(), 1);
}
