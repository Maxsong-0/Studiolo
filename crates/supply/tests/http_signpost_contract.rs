//! HTTP 与 MCP 共用路牌预算；续读入口不能为了通过预算而被裁剪掉。
use studiolo_supply::{
    HARD_BUDGET_TOKENS, Principal, SignpostInput, Snapshot, TARGET_BUDGET_TOKENS, count_tokens,
    signpost,
};

fn principal() -> Principal {
    Principal {
        agent_type: "claude".into(),
        instance_id: "01JT3STUDIOLO0000000000001".into(),
        registered_at: "2026-09-10T01:00:00Z".into(),
    }
}

#[test]
fn trimming_keeps_http_routes_and_required_headers_inside_the_measured_budget() {
    let snapshot = Snapshot::default();
    let principal = principal();
    let description = "可裁剪说明：按需读取当前白名单项目。".repeat(3000);
    let tools = [
        (
            "Authorization: Bearer <类型令牌>；Studiolo-Session-Id: <握手响应头值>",
            "所有续读请求携带这两个请求头",
        ),
        ("GET /read/handshake", description.as_str()),
        ("GET /read/entries?project=…", "列目录"),
        ("GET /read/note?path=…", "读正文"),
        (
            "GET /read/search?project=…&text=…&mode=full_text|path",
            "搜索",
        ),
    ];
    let input = SignpostInput {
        snapshot: &snapshot,
        allow: &[],
        principal: &principal,
        generation: 1,
        generated_at: "2026-09-10T01:00:00Z",
        freshness: None,
        tools: &tools,
        policy: None,
        rules_whitelist: None,
    };
    let mcp = signpost::derive(&input).unwrap();
    let http = signpost::derive_http(&input).unwrap();
    for result in [&mcp, &http] {
        assert_eq!(result.trim_level, 1, "超长说明必须触发裁剪");
        assert!(result.tokens <= TARGET_BUDGET_TOKENS);
        assert!(result.tokens <= HARD_BUDGET_TOKENS);
        assert_eq!(result.tokens, count_tokens(&result.text));
        assert!(!result.text.contains("可裁剪说明"));
        for (entry, _) in tools {
            assert!(result.text.contains(entry), "续读入口被裁剪：{entry}");
        }
    }
    assert!(http.text.contains("HTTP 入口："));
    assert!(!http.text.contains("tools："));
    assert_eq!(http.text.replacen("HTTP 入口：", "tools：", 1), mcp.text);
}

#[test]
fn oversized_nontrimmable_entry_fails_closed_for_both_shells() {
    let snapshot = Snapshot::default();
    let principal = principal();
    let required_entry = "GET /read/handshake Authorization Studiolo-Session-Id ".repeat(1000);
    assert!(count_tokens(&required_entry) > HARD_BUDGET_TOKENS);
    let tools = [(required_entry.as_str(), "不可裁剪的续读入口")];
    let input = SignpostInput {
        snapshot: &snapshot,
        allow: &[],
        principal: &principal,
        generation: 1,
        generated_at: "2026-09-10T01:00:00Z",
        freshness: None,
        tools: &tools,
        policy: None,
        rules_whitelist: None,
    };
    for result in [signpost::derive(&input), signpost::derive_http(&input)] {
        let exceeded = result.expect_err("超预算必须失败，不能静默删除入口或返回超长路牌");
        assert_eq!(exceeded.limit, HARD_BUDGET_TOKENS);
        assert!(exceeded.tokens > exceeded.limit);
    }
}
