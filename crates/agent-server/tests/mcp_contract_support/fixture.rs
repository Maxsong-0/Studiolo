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
    fixture_with_router(serve::router).await
}

async fn fixture_with_router(make_router: impl FnOnce(Arc<AppState>) -> axum::Router) -> Fixture {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
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
        index_cache: None,
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
    let app = make_router(state.clone());
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
