// 实例登记审计（T-264）：服务器实际应答的协议版本，和客户端自报的名字、版本。父实例那一格在 `policy_contract.rs`。

impl Fixture {
    /// 用指定的协议版本和 `clientInfo` 握手。首包照规范不带版本头：带了就得和正文一致，测不了对不上的。
    async fn initialize_as(&self, version: &str, client: Value) -> (String, Value) {
        let response = self
            .client
            .post(&self.base)
            .bearer_auth(&self.token)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": version, "capabilities": {}, "clientInfo": client
                }})
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        let (session, reply) = read_rpc(response, 1, version).await;
        (session.unwrap(), reply)
    }

    /// 最近一条登记审计的内容。
    fn registered(&self) -> Value {
        self.ledger()
            .query(&AuditFilter::default().with_action("agent_instance.registered"))
            .unwrap()
            .last()
            .unwrap()
            .event()
            .payload()
            .as_value()
            .clone()
    }
}

#[tokio::test]
async fn registration_records_the_version_the_server_answered_not_the_one_the_client_asked_for() {
    let f = fixture().await;
    for (asked, answered) in [
        ("2025-11-25", "2025-11-25"),
        ("2025-06-18", "2025-06-18"),
        ("2025-03-26", "2025-03-26"),
        // 不在表里的：更旧的、无会话的新版、没见过的，一律退到 2025-11-25。
        ("2024-11-05", "2025-11-25"),
        ("2026-07-28", "2025-11-25"),
        ("2099-01-01", "2025-11-25"),
        ("随便写的", "2025-11-25"),
    ] {
        let (session, response) = f
            .initialize_as(
                asked,
                json!({"name": "claude-code", "title": "不记标题", "version": "2.1.332"}),
            )
            .await;
        assert_eq!(response["result"]["protocolVersion"], answered, "{asked}");
        let registered = f.registered();
        let instance = response["result"]["instructions"]
            .as_str()
            .unwrap()
            .split("实例 ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        // 整条相等：除了这五格什么都没有，正文、令牌、会话号都不在里面。
        assert_eq!(
            registered,
            json!({
                "agent_type": "claude",
                "agent_instance": instance,
                "parent": null,
                "negotiated_version": answered,
                "client_reported": {"name": "claude-code", "version": "2.1.332"},
            }),
            "{asked}"
        );
        assert!(!registered.to_string().contains(&session));
    }
}

#[tokio::test]
async fn an_over_long_client_name_is_cut_to_sixty_four_characters() {
    let f = fixture().await;
    f.initialize_as(
        "2025-11-25",
        json!({"name": "长".repeat(20_000), "version": "9".repeat(65)}),
    )
    .await;
    assert_eq!(
        f.registered()["client_reported"],
        json!({"name": format!("{}…", "长".repeat(63)), "version": format!("{}…", "9".repeat(63))})
    );
    // 恰好 64 个字符不算超长；空的照实记空。
    f.initialize_as(
        "2025-11-25",
        json!({"name": "长".repeat(64), "version": ""}),
    )
    .await;
    assert_eq!(
        f.registered()["client_reported"],
        json!({"name": "长".repeat(64), "version": ""})
    );
}

#[tokio::test]
async fn control_and_invisible_characters_in_a_client_name_never_reach_the_ledger() {
    let f = fixture().await;
    f.initialize_as(
        "2025-11-25",
        json!({
            "name": "cla\u{0}ude\n\u{1b}[31m\u{85}-\u{202e}code\u{200b}",
            "version": "\t1.0\r\n\u{2028}2\u{7f}",
        }),
    )
    .await;
    assert_eq!(
        f.registered()["client_reported"],
        json!({"name": "claude[31m-code", "version": "1.02"})
    );
    // 去掉的字符不占名额：前面垫满控制字符，后面的名字照样留得下。
    f.initialize_as(
        "2025-11-25",
        json!({"name": format!("{}codex", "\u{1}".repeat(5_000)), "version": "0"}),
    )
    .await;
    assert_eq!(f.registered()["client_reported"]["name"], "codex");
}

#[tokio::test]
async fn what_a_client_calls_itself_changes_no_decision() {
    let f = fixture().await;
    let mut outcomes = Vec::new();
    for name in ["contract", "codex", "admin", "not_negotiated:connector"] {
        let (session, _) = f
            .initialize_as("2025-11-25", json!({"name": name, "version": "0"}))
            .await;
        let registered = f.registered();
        // 主体只由令牌定；自报名是对象，装不成没有协商的那两条路。
        assert_eq!(registered["agent_type"], "claude", "{name}");
        assert_eq!(registered["negotiated_version"], "2025-11-25", "{name}");
        assert_eq!(registered["client_reported"]["name"], name);
        let mut seen = Vec::new();
        for path in ["项目/Alpha/00-总览/INDEX.md", "School/secret.md"] {
            let reply = f
                .call(&f.token, &session, 2, "read_note", json!({"path": path}))
                .await;
            seen.push((is_error(&reply), tool_text(&reply), reply["error"].clone()));
        }
        assert!(!seen[0].0 && seen[1].0, "{name}");
        outcomes.push(seen);
    }
    assert!(outcomes.windows(2).all(|pair| pair[0] == pair[1]));
}
