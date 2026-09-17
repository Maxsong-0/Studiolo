#[tokio::test]
async fn duplicate_session_headers_and_stateless_protocol_cannot_bypass_binding() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    for method in [reqwest::Method::POST, reqwest::Method::GET, reqwest::Method::DELETE] {
        assert_eq!(transport(&f, method, &f.token, &session).header("Mcp-Session-Id", &session)
            .send().await.unwrap().status(), 400);
    }
    let response = f.client.post(&f.base).bearer_auth(&f.token).header("Mcp-Session-Id", &session)
        .header("Accept", "application/json, text/event-stream").header("Content-Type", "application/json")
        .header("MCP-Protocol-Version", "2026-07-28")
        .body(json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}
        }}).to_string()).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(response.headers().get("mcp-session-id").is_none());
    // 即便沿用旧协议头，也不能在已有 SID 上重复 initialize 来绕开新建速率。
    let repeated = f.request(Some(&f.token), Some(&session), json!({"jsonrpc":"2.0","id":2,"method":"initialize",
        "params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}}
    })).send().await.unwrap();
    assert_eq!(repeated.status(), 400);
    assert_eq!(f.ledger().query(&AuditFilter::default().with_action("agent_instance.registered")).unwrap().len(), 1);
    assert!(f.state.degraded().is_none());
}
