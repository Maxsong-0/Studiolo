// 没认出类型的拒绝（T-241）：登记限频，答复和以前一个字不差；已认证类型的拒绝照旧逐条登记。

/// 一次答复里客户端看得到的全部：状态码、challenge、正文。
async fn answer(request: reqwest::RequestBuilder) -> (u16, Option<String>, String) {
    let response = request.send().await.unwrap();
    let challenge = response
        .headers()
        .get("www-authenticate")
        .map(|value| value.to_str().unwrap().to_owned());
    (
        response.status().as_u16(),
        challenge,
        response.text().await.unwrap(),
    )
}

/// 账本里的拒绝，按次序：（类型，入口，原因）。
fn denials(f: &Fixture) -> Vec<(Option<String>, String, String)> {
    f.ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|record| {
            let payload = record.event().payload().as_value().clone();
            let text = |key: &str| payload[key].as_str().map(str::to_owned);
            (
                text("agent_type"),
                text("action").unwrap(),
                text("reason").unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn unrecognized_callers_are_recorded_once_per_entrance_with_every_answer_unchanged() {
    let f = fixture().await;
    let read = f.base.replace("/mcp", "/read");
    let anonymous = |entrance: &str, reason: &str| (None, entrance.to_owned(), reason.to_owned());
    let refused = (
        401,
        Some("Bearer realm=\"studiolo-agent\"".to_owned()),
        String::new(),
    );
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    for round in 0..6 {
        for token in [None, Some(""), Some("sag1_wrong")] {
            assert_eq!(answer(f.request(token, None, list.clone())).await, refused);
            // 路径不许成为名下的一部分：换着路径来也只有一条。
            for path in ["handshake".to_owned(), format!("no-such-{round}")] {
                let mut request = f.client.get(format!("{read}/{path}"));
                if let Some(token) = token {
                    request = request.header("Authorization", format!("Bearer {token}"));
                }
                assert_eq!(answer(request).await, refused, "{path}");
            }
        }
    }
    let mut expected = vec![
        anonymous("mcp.request", "unauthenticated"),
        anonymous("http.request", "unauthenticated"),
    ];
    assert_eq!(denials(&f), expected);
    // 身份库读不出来是另一个原因：503、不带 challenge，第一次和后面几次一样。
    let database = f.data.join("agent.sqlite3");
    fs::set_permissions(&database, fs::Permissions::from_mode(0o640)).unwrap();
    let unavailable = (503, None, "身份校验暂不可用，请稍后重试".to_owned());
    for _ in 0..5 {
        assert_eq!(
            answer(f.request(Some(&f.token), None, list.clone())).await,
            unavailable
        );
        let handshake = f.client.get(format!("{read}/handshake"));
        assert_eq!(answer(handshake.bearer_auth(&f.token)).await, unavailable);
    }
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    expected.extend([
        anonymous("mcp.request", "identity_unavailable"),
        anonymous("http.request", "identity_unavailable"),
    ]);
    assert_eq!(denials(&f), expected);
}

#[tokio::test]
async fn a_read_path_named_like_the_mcp_entrance_does_not_use_up_its_minute() {
    let f = fixture().await;
    let anonymous = |entrance: &str| (None, entrance.to_owned(), "unauthenticated".to_owned());
    let read = f.base.replace("/mcp", "/read/mcp");
    for _ in 0..3 {
        assert_eq!(answer(f.client.get(&read)).await.0, 401);
    }
    assert_eq!(denials(&f), [anonymous("http.request")]);
    assert_eq!(answer(f.client.get(&f.base)).await.0, 401);
    assert_eq!(
        denials(&f),
        [anonymous("http.request"), anonymous("mcp.request")]
    );
}

#[tokio::test]
async fn recognized_types_are_still_recorded_every_time_they_are_denied() {
    let f = fixture().await;
    let read = f.base.replace("/mcp", "/read");
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let mut expected = Vec::new();
    for _ in 0..3 {
        let (status, _, _) = answer(f.request(
            Some(&f.token),
            Some("01T241NOSUCHSESSION0000000"),
            list.clone(),
        ))
        .await;
        assert_eq!(status, 404);
        expected.push(("mcp.request", "session_invalid"));
        let entries = f.client.get(format!("{read}/entries?project=x"));
        assert_eq!(answer(entries.bearer_auth(&f.token)).await.0, 401);
        expected.push(("http.request", "missing_session"));
    }
    let expected = expected
        .into_iter()
        .map(|(entrance, reason)| {
            (
                Some("claude".to_owned()),
                entrance.to_owned(),
                reason.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(denials(&f), expected);
}
