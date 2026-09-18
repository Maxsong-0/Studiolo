//! HTTP 契约共用私有临时目录、真实 listener 与断言。
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_supply::AgentStore;
use tempfile::TempDir;

pub struct HttpServer {
    pub data: PathBuf,
    pub token: String,
    pub state: Arc<AppState>,
    base: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

pub async fn spawn(options: ServeOptions) -> HttpServer {
    for dir in [&options.data, &options.data.join("audit")] {
        fs::create_dir_all(dir).unwrap();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let store = AgentStore::open(&options.data).unwrap();
    let token = store
        .issue_type("claude", "2026-09-10T00:00:00Z")
        .unwrap()
        .token;
    drop(store);
    drop(SqliteAuditLedger::open(&options.data.join("audit")).unwrap());
    let state = serve::prepare(&options).unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = serve::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    HttpServer {
        data: options.data,
        token,
        state,
        base: format!("http://127.0.0.1:{port}"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
        server,
    }
}

pub async fn fixture() -> (TempDir, HttpServer) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 进行中 |\n\n## 当前重点\n\n- **T-037 HTTP 兜底**：冷启动路牌。\n",
    );
    write(
        &vault.join("项目/Alpha/04-协调/TASKS.md"),
        "# 协调\n\n## ① 进行中\n\n| ID | 任务 | 负责线 | 文件 | 状态 | 日期 |\n|---|---|---|---|---|---|\n| T-037 | HTTP 兜底 | Codex | agent-server | 进行中 | 2026-09-10 |\n\n## ② 占用\n",
    );
    write(
        &vault.join("项目/Alpha/02-日志/2026-09-10.md"),
        "# 日志\n\nAlpha 的事。\n",
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
    let server = spawn(ServeOptions {
        index_cache: None,
        vault: Some(vault),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into(), "_规则".into()],
        data: temp.path().join("data"),
        policy: None,
        port: 0,
    })
    .await;
    (temp, server)
}

impl HttpServer {
    pub fn request(
        &self,
        method: reqwest::Method,
        route: &str,
        token: Option<&str>,
        session: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut request = self.client.request(method, format!("{}{route}", self.base));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(session) = session {
            request = request.header("Studiolo-Session-Id", session);
        }
        request
    }

    pub fn get(&self, route: &str, session: Option<&str>) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::GET, route, Some(&self.token), session)
    }

    pub async fn handshake(&self) -> (String, String, String) {
        let response = self.get("/read/handshake", None).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_headers(&response, true);
        let session = header(&response, "studiolo-session-id");
        let instance = header(&response, "studiolo-instance-id");
        assert!(!session.is_empty());
        assert!(ulid::Ulid::from_string(&instance).is_ok());
        (session, instance, response.text().await.unwrap())
    }

    pub fn store(&self) -> AgentStore {
        AgentStore::open_existing(&self.data).unwrap()
    }

    pub fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }
}

pub fn header(response: &reqwest::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .unwrap()
        .to_str()
        .unwrap()
        .into()
}

pub fn assert_headers(response: &reqwest::Response, success: bool) {
    assert_eq!(header(response, "cache-control"), "no-store");
    assert_eq!(header(response, "x-content-type-options"), "nosniff");
    assert!(
        response
            .headers()
            .keys()
            .all(|name| !name.as_str().starts_with("access-control-"))
    );
    if success {
        assert_eq!(
            header(response, "content-type").replace(' ', ""),
            "text/plain;charset=utf-8"
        );
    }
}

pub async fn denied(response: reqwest::Response, status: u16) -> String {
    assert_eq!(response.status(), status);
    assert_headers(&response, false);
    if status == 401 {
        assert!(response.headers().get("www-authenticate").is_some());
    }
    let body = response.text().await.unwrap();
    for secret in ["秘密内容", "Alpha · 总览", "Alpha 的事", "永远说中文"] {
        assert!(!body.contains(secret), "error leaked {secret}: {body}");
    }
    body
}
