#[tokio::test]
async fn initialize_serves_a_budgeted_signpost_and_tools_read_only_inside_the_allow_list() {
    let f = fixture().await;
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("# Studiolo 路牌（代次 1"));
    assert!(instructions.contains("类型 claude · 实例 "));
    assert!(instructions.contains("- 项目/Alpha："));
    assert!(instructions.contains("进行中：项目/Alpha · T-033 M3 首片（进行中）"));
    assert!(instructions.contains("1 永远说中文"));
    assert!(instructions.contains("tools：handshake"));
    assert!(!instructions.contains("秘密") && !instructions.contains("Beta"));
    assert!(count_tokens(instructions) <= HARD_BUDGET_TOKENS);
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    let instance = instructions
        .split("实例 ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_owned();

    let tools = f
        .rpc(&f.token, Some(&session), 2, "tools/list", json!({}))
        .await
        .1;
    let mut names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "board",
            "handshake",
            "list_entries",
            "list_proposals",
            "policy_read",
            "propose_desired",
            "read_note",
            "search"
        ]
    );

    let read = f
        .call(
            &f.token,
            &session,
            3,
            "read_note",
            json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(!is_error(&read), "{read}");
    let text = tool_text(&read);
    assert!(text.starts_with("revision "));
    assert!(text.contains("# Alpha · 总览"));

    for (path, reason) in [
        ("School/secret.md", "outside_allowlist"),
        ("项目/Beta/00-总览/INDEX.md", "outside_allowlist"),
        ("../School/secret.md", "unsafe_path"),
        ("项目/Alpha/missing.md", "not_found"),
    ] {
        let denied = f
            .call(&f.token, &session, 10, "read_note", json!({"path": path}))
            .await;
        assert!(is_error(&denied), "{path}: {denied}");
        assert!(!denied.to_string().contains("秘密内容"));
        let events = f
            .ledger()
            .query(&AuditFilter::default().with_action("agent.action_denied"))
            .unwrap();
        assert_eq!(
            events.last().unwrap().event().payload().as_value()["reason"],
            reason
        );
    }
    assert!(is_error(
        &f.call(
            &f.token,
            &session,
            11,
            "list_entries",
            json!({"project": "School"})
        )
        .await
    ));

    let listed = f
        .call(
            &f.token,
            &session,
            4,
            "list_entries",
            json!({"project": "项目/Alpha"}),
        )
        .await;
    let listed = tool_text(&listed);
    assert!(listed.contains("note\t项目/Alpha/00-总览/INDEX.md\t"));
    assert!(!listed.contains("School") && !listed.contains("Beta"));

    let found = f
        .call(
            &f.token,
            &session,
            5,
            "search",
            json!({"project": "项目/Alpha", "text": "Alpha 的事"}),
        )
        .await;
    let found = tool_text(&found);
    assert!(
        found.contains("项目/Alpha/02-日志/2026-09-06.md:3\t"),
        "{found}"
    );
    assert!(is_error(
        &f.call(
            &f.token,
            &session,
            6,
            "search",
            json!({"project": "项目/Alpha", "text": "x", "mode": "regex"})
        )
        .await
    ));

    // 审计先于返回：账本与派生索引都能按类型/实例/路径查到这次读取。
    let reads = f
        .ledger()
        .query(&AuditFilter::default().with_action("content.read"))
        .unwrap();
    let read_paths = reads
        .iter()
        .map(|record| {
            record.event().payload().as_value()["path"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(read_paths.contains(&"项目/Alpha/00-总览/INDEX.md".to_owned()));
    assert!(read_paths.contains(&"项目/Alpha".to_owned()));
    assert!(read_paths.contains(&"项目/Alpha/02-日志/2026-09-06.md".to_owned()));
    assert!(
        reads
            .iter()
            .all(|record| record.event().payload().as_value()["agent_instance"] == json!(instance))
    );
    let indexed = f
        .store()
        .query_reads(&ReadQuery {
            agent_type: Some("claude".into()),
            instance_id: Some(instance.clone()),
            path_prefix: Some("项目/Alpha/00-总览/".into()),
            limit: 10,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(indexed.len(), 1);
    assert_eq!(indexed[0].tool, "read_note");
    assert!(
        reads
            .iter()
            .any(|record| record.offset() == indexed[0].ledger_offset)
    );
    let served = f
        .ledger()
        .query(&AuditFilter::default().with_action("handshake.served"))
        .unwrap();
    assert_eq!(served.len(), 1);
    assert_eq!(
        served[0].event().payload().as_value()["token_count"],
        json!(count_tokens(instructions))
    );
    assert_eq!(
        f.ledger()
            .query(&AuditFilter::default().with_action("agent_instance.registered"))
            .unwrap()
            .len(),
        1
    );

    // 同会话重取路牌；吊销类型后即刻拒绝，新握手 401。
    assert!(
        tool_text(&f.call(&f.token, &session, 7, "handshake", json!({})).await)
            .starts_with("# Studiolo 路牌")
    );
    assert!(
        f.store()
            .revoke_type("claude", "2026-09-07T01:00:00Z")
            .unwrap()
    );
    // 同一会话的下一次调用在门禁处就被拒绝：类型吊销即刻使其全部实例失效。
    let status = f
        .request(Some(&f.token), Some(&session), json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {"name": "read_note", "arguments": {"path": "项目/Alpha/00-总览/INDEX.md"}}}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 401);
    assert_eq!(
        f.ledger()
            .query(&AuditFilter::default().with_action("agent.action_denied"))
            .unwrap()
            .last()
            .unwrap()
            .event()
            .payload()
            .as_value()["reason"],
        "unauthenticated"
    );
    let status = f
        .request(Some(&f.token), None, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "x", "version": "0"}}}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 401);
}
