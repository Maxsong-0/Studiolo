//! 真实 loopback 契约：raw JSON-RPC over Streamable HTTP（reqwest 0.12，不引入第二个 reqwest）。
//! 覆盖：401、初始化路牌与硬预算、tools/list、read_note 正反、list_entries、search、吊销即刻生效、
//! 审计先于返回与读取索引四维查询、审计不可用时的降级握手。

use std::{
    fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::{AgentStore, HARD_BUDGET_TOKENS, ReadQuery, count_tokens};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    data: std::path::PathBuf,
    token: String,
    base: String,
    client: reqwest::Client,
    state: Arc<AppState>,
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

async fn fixture() -> Fixture {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 🟡 进行中 |\n| 最近日志 | [[2026-09-06]] |\n| 未关闭 BUG | 无 |\n\n## 当前重点\n\n- **T-033 M3 首片**：冷启动路牌。\n",
    );
    write(
        &vault.join("项目/Alpha/04-协调/TASKS.md"),
        "# 协调\n\n## ① 进行中\n\n| ID | 任务 | 负责线 | 文件 | 状态 | 日期 |\n|---|---|---|---|---|---|\n| T-033 | M3 首片 | Claude | crates/supply | 进行中 | 2026-09-06 |\n\n## ② 占用\n",
    );
    write(
        &vault.join("项目/Alpha/02-日志/2026-09-06.md"),
        "# 日志\n\n做了 Alpha 的事。\n",
    );
    write(
        &vault.join("_规则/通用工作规则.md"),
        "# 规则\n\n## 准则① 永远说中文\n\n正文。\n",
    );
    write(&vault.join("School/secret.md"), "# 私人成绩 秘密内容\n");
    write(
        &vault.join("项目/Beta/00-总览/INDEX.md"),
        "# Beta 不在白名单\n",
    );
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));
    let store = AgentStore::open(&data).unwrap();
    let token = store
        .issue_type("claude", "2026-09-07T00:00:00Z")
        .unwrap()
        .token;
    drop(store);
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());

    let state = serve::prepare(&ServeOptions {
        vault: Some(vault.clone()),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into(), "_规则".into()],
        data: data.clone(),
        port: 0,
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = serve::router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        _temp: temp,
        data,
        token,
        base: format!("http://127.0.0.1:{}/mcp", addr.port()),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
        state,
    }
}

impl Fixture {
    fn request(
        &self,
        token: Option<&str>,
        session: Option<&str>,
        body: Value,
    ) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    }

    /// 发一个 JSON-RPC 请求，返回（会话 id 头，响应对象）；SSE 按 `data:` 事件解析，读到目标 id 即停。
    async fn rpc(
        &self,
        token: &str,
        session: Option<&str>,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Option<String>, Value) {
        let mut response = self
            .request(
                token.into(),
                session,
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{method}");
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .map(|value| value.to_str().unwrap().to_owned());
        let content_type = response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let mut buffer = String::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            if content_type.starts_with("application/json") {
                if let Ok(value) = serde_json::from_str::<Value>(&buffer) {
                    return (session_id, value);
                }
                continue;
            }
            for event in buffer.split("\n\n") {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join("");
                if data.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&data)
                    && value.get("id") == Some(&json!(id))
                {
                    return (session_id, value);
                }
            }
        }
        panic!("no JSON-RPC response for {method}: {buffer}");
    }

    async fn initialize(&self, token: &str) -> (String, Value) {
        let (session, response) = self
            .rpc(
                token,
                None,
                1,
                "initialize",
                json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "contract", "version": "0"}}),
            )
            .await;
        let session = session.expect("session id header");
        let status = self
            .request(
                token.into(),
                Some(&session),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 202);
        (session, response)
    }

    async fn call(
        &self,
        token: &str,
        session: &str,
        id: u64,
        name: &str,
        arguments: Value,
    ) -> Value {
        self.rpc(
            token,
            Some(session),
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
        .1
    }

    fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }

    fn store(&self) -> AgentStore {
        AgentStore::open_existing(&self.data).unwrap()
    }
}

fn tool_text(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn is_error(response: &Value) -> bool {
    response.get("error").is_some() || response["result"]["isError"] == json!(true)
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected_before_any_content_and_audited() {
    let f = fixture().await;
    for token in [None, Some("sag1_wrong"), Some("")] {
        let response = f
            .request(token, None, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "x", "version": "0"}}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert!(response.headers().get("www-authenticate").is_some());
        assert!(response.text().await.unwrap().is_empty());
    }
    let denied = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap();
    assert_eq!(denied.len(), 3);
    assert_eq!(
        denied[0].event().payload().as_value()["reason"],
        "unauthenticated"
    );
    assert!(denied[0].event().payload().as_value()["agent_type"].is_null());
}

#[tokio::test]
async fn initialize_serves_a_budgeted_signpost_and_tools_read_only_inside_the_allow_list() {
    let f = fixture().await;
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("# Studiolo 路牌（代次 1"));
    assert!(instructions.contains("类型 claude · 实例 "));
    assert!(instructions.contains("### 项目/Alpha\n当前版本：v0.3"));
    assert!(instructions.contains("进行中：T-033 M3 首片（进行中）"));
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
    assert_eq!(names, ["handshake", "list_entries", "read_note", "search"]);

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

#[tokio::test]
async fn audit_unavailable_degrades_the_handshake_without_any_vault_content() {
    let f = fixture().await;
    // 破坏审计私有目录权限：账本存储复核失败 → 追加失败 → 降级握手。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    assert!(
        f.state
            .ledger
            .append(
                studiolo_supply::audit::handshake_degraded(
                    studiolo_supply::audit::Actor::anonymous(),
                    "probe"
                )
                .unwrap()
            )
            .is_err(),
        "ledger storage check must fail closed after the directory mode changed"
    );
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("审计不可用：outage "));
    // 路牌六项一项不给：没有项目状态、待办、准则、入口，也没有任何 vault 内容。
    for leaked in [
        "# Studiolo 路牌",
        "## 1 我是谁",
        "Alpha",
        "准则",
        "永远说中文",
        "入口：",
        "tools：",
    ] {
        assert!(
            !instructions.contains(leaked),
            "degraded handshake leaked {leaked}: {instructions}"
        );
    }
    let denied = f
        .call(
            &f.token,
            &session,
            2,
            "read_note",
            json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(is_error(&denied));
    assert!(!denied.to_string().contains("Alpha · 总览"));
    let outage_id = instructions
        .trim_start_matches("审计不可用：outage ")
        .split('。')
        .next()
        .unwrap()
        .to_owned();
    assert!(f.state.degraded().is_some());
    // 恢复后：新握手正常，outage 记录已结束且带拒绝计数。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    let (_, recovered) = f.initialize(&f.token).await;
    assert!(
        recovered["result"]["instructions"]
            .as_str()
            .unwrap()
            .starts_with("# Studiolo 路牌")
    );
    assert!(f.state.degraded().is_none());
    let outages = f.store().outages().unwrap();
    assert_eq!(outages.len(), 1);
    assert_eq!(outages[0].0, outage_id);
    assert!(outages[0].2.is_some(), "outage ended");
    assert!(
        outages[0].3 >= 2,
        "instance registration + read were denied: {}",
        outages[0].3
    );
}
