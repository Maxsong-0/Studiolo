//! 真实 loopback MCP 副本服务夹具。
use crate::replica_support::{ReplicaFixture, private_dir};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_supply::AgentStore;

pub struct Fixture {
    pub replica: ReplicaFixture,
    pub data: PathBuf,
    pub token: String,
    base: String,
    client: reqwest::Client,
    pub state: Arc<AppState>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub async fn fixture() -> Fixture {
    fixture_with_age(10_000).await
}

pub async fn fixture_with_age(age_ms: i64) -> Fixture {
    let mut replica = ReplicaFixture::new();
    replica.observed = jiff::Timestamp::now().as_millisecond() - age_ms;
    replica.publish(crate::replica_support::FIRST, "oldmarker", replica.observed);
    let data = replica.temp.path().join("agent-data");
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
        vault: None,
        replica_root: Some(replica.root.clone()),
        replica_max_age_secs: 300,
        allow: vec!["project".into()],
        data: data.clone(),
        port: 0,
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = serve::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        replica,
        data,
        token,
        state,
        server,
        base: format!("http://127.0.0.1:{port}/mcp"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
    }
}

impl Fixture {
    pub fn request(
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
    pub async fn rpc(
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

    pub async fn initialize(&self, token: &str) -> (String, Value) {
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

    pub async fn call(
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

    pub fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }

    pub fn store(&self) -> AgentStore {
        AgentStore::open_existing(&self.data).unwrap()
    }
}

pub fn tool_text(response: &Value) -> String {
    response["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|content| content["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn is_error(response: &Value) -> bool {
    response.get("error").is_some() || response["result"]["isError"] == json!(true)
}
