//! 停服契约（T-184，vault BUG-033）：实际产品二进制收到 SIGTERM 后在规定时间内干净退出。
//!
//! 2026-10-15 生产上旧版 agent 收到 SIGTERM 20 秒没退出，被 systemd 强杀：axum 的优雅关停要等每条
//! 连接自己结束，客户端开着的 MCP GET SSE 流却不会自己结束。这里起真实 `serve` 进程复现，并钉住契约
//! `docs/contracts/agent-supply.md` §9：关停期间不再接受新会话，已开始的供给完成后退出。
//!
//! 部署形态与生产一致（`--policy` 加 `--board-socket`）；黑板是本进程里的假服务，用来把一次工具调用
//! 确定地卡在半路。夹具只用独立临时目录与虚构笔记，绝不含真实种子、token 或口令。
#![cfg(unix)]

use std::{
    fs,
    io::{BufRead as _, BufReader, Read as _},
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_policy::{DesiredInput, DesiredStatus, PolicyKind, PolicyStore, Submitter};
use studiolo_supply::AgentStore;
use tempfile::TempDir;
use tokio::sync::Notify;

include!("support/agent_binary.rs");

const NOW: &str = "2026-10-15T00:00:00Z";
/// systemd 单元给 20 秒（`TimeoutStopSec=20`）；干净关停要远小于它。
const PROMPT_EXIT: Duration = Duration::from_secs(5);
/// 等子进程退出的最长时间：到点仍不退就 SIGKILL 收尸并判失败，测试自己不会挂住。
const GIVE_UP: Duration = Duration::from_secs(10);
/// 黑板调用的结果里带这个标记，证明卡在半路的那次供给最终完整送到了客户端。
const CANARY: &str = "SHUTDOWN_DRAIN_CANARY";

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("有父目录")).expect("建立虚构目录");
    fs::write(path, body).expect("写入虚构笔记");
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).expect("建立私有目录");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("收紧私有目录权限");
}

/// 给 claude 发布一份只读 `项目/Alpha` 的能力集，和生产一样经策略平面授权。
fn publish_capability(policy: &Path) {
    let store = PolicyStore::open(policy).expect("初始化策略库");
    let body = json!({
        "ai_type": "claude",
        "read": {"project_whitelist": ["项目/Alpha"], "deny_by_default": true, "rules_whitelist": ["通用工作规则.md"]},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": false, "activate": false},
        "hard_bound_skills": [],
    })
    .to_string();
    let record = store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: "claude".into(),
                body: Some(body),
            },
            Submitter::Owner,
            NOW,
        )
        .expect("提交能力集");
    assert_eq!(
        record.status,
        DesiredStatus::Validated,
        "{:?}",
        record.report
    );
    let hash = store.effective().expect("读取 effective").effective_hash;
    store
        .activate(&record.id, &hash, "owner", NOW)
        .expect("发布能力集");
}

/// 假黑板：`list` 立即回空表；其余操作先报「到了」，再等测试放行，才回带标记的结果。
struct FakeBoard {
    socket: PathBuf,
    reached: Arc<Notify>,
    release: Arc<Notify>,
    server: tokio::task::JoinHandle<()>,
}

impl FakeBoard {
    fn start(directory: &Path) -> Self {
        let socket = directory.join("board.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("绑定假黑板 socket");
        let reached = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let (entered, allowed) = (reached.clone(), release.clone());
        let app = axum::Router::new().route(
            "/internal/board",
            axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
                let (entered, allowed) = (entered.clone(), allowed.clone());
                async move {
                    if request["operation"] == "list" {
                        return axum::Json(json!([]));
                    }
                    entered.notify_one();
                    allowed.notified().await;
                    axum::Json(json!({"task_id": "fixture", "note": CANARY}))
                }
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("假黑板服务");
        });
        Self {
            socket,
            reached,
            release,
            server,
        }
    }
}

impl Drop for FakeBoard {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// 一个真实运行的 `studiolo-agent serve`：独立临时目录、虚构笔记、回环端口、假黑板。
struct Agent {
    child: Child,
    client: Client,
    output: Arc<Mutex<String>>,
    board: FakeBoard,
    // 端口占位在 Drop 收走子进程之后才释放（T-171）；临时目录最后清理。
    _reservation: tokio::net::TcpSocket,
    _temp: TempDir,
}

impl Agent {
    fn start() -> Self {
        let temp = TempDir::new_in(fs::canonicalize("/tmp").expect("/tmp 可解析"))
            .expect("建立独立临时目录");
        let vault = temp.path().join("vault");
        write(
            &vault.join("项目/Alpha/00-总览/INDEX.md"),
            "# Alpha · 总览\n\n虚构内容，不是秘密。\n",
        );
        write(
            &vault.join("_规则/通用工作规则.md"),
            "# 规则\n\n虚构规则正文。\n",
        );
        let data = temp.path().join("agent-data");
        private_dir(&data);
        private_dir(&data.join("audit"));
        let store = AgentStore::open(&data).expect("初始化 agent 私有库");
        let token = store.issue_type("claude", NOW).expect("签发类型令牌").token;
        drop(store);
        drop(SqliteAuditLedger::open(&data.join("audit")).expect("初始化审计账本"));
        let policy = temp.path().join("policy-data");
        private_dir(&policy);
        publish_capability(&policy);
        let board = FakeBoard::start(temp.path());

        // 端口占位与子进程同寿（T-171）：带 SO_REUSEADDR 绑在 0.0.0.0、不监听，产品绑 127.0.0.1 的同一端口。
        let reservation = tokio::net::TcpSocket::new_v4().expect("建立端口占位");
        reservation
            .set_reuseaddr(true)
            .expect("占位须允许产品共享绑定");
        reservation
            .bind(([0, 0, 0, 0], 0).into())
            .expect("取一个空闲端口");
        let port = reservation.local_addr().expect("占位有本地地址").port();
        let path = |path: &Path| path.to_str().expect("临时目录路径是 UTF-8").to_owned();
        let mut child = Command::new(agent_binary())
            .args(["serve", "--vault", &path(&vault)])
            .args(["--allow", "项目/Alpha", "--allow", "_规则"])
            .args(["--data", &path(&data), "--policy", &path(&policy)])
            .args(["--board-socket", &path(&board.socket)])
            .args(["--port", &port.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("启动真实 serve 进程");

        let output = Arc::new(Mutex::new(String::new()));
        let stderr = child.stderr.take().expect("serve 标准错误已接管");
        let collected = output.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            collected.lock().expect("诊断缓冲").push_str(&text);
        });
        // 等产品自己宣布已在监听；之后另起线程把标准输出读到底，关停时的输出不会因管道关闭而写失败。
        let mut stdout = BufReader::new(child.stdout.take().expect("serve 标准输出已接管"));
        let ready = format!("http://127.0.0.1:{port}/mcp");
        loop {
            let mut line = String::new();
            let read = stdout.read_line(&mut line).expect("读取 serve 启动输出");
            output.lock().expect("诊断缓冲").push_str(&line);
            if read == 0 {
                let _ = child.kill();
                let status = child.wait().ok();
                std::thread::sleep(Duration::from_millis(200));
                panic!(
                    "serve 在宣布监听之前就结束了（退出 {status:?}）：\n{}",
                    output.lock().expect("诊断缓冲")
                );
            }
            if line.contains(&ready) {
                break;
            }
        }
        let collected = output.clone();
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                let mut collected = collected.lock().expect("诊断缓冲");
                collected.push_str(&line);
                collected.push('\n');
            }
        });
        Self {
            child,
            client: Client { base: ready, token },
            output,
            board,
            _reservation: reservation,
            _temp: temp,
        }
    }

    fn terminate(&self) {
        rustix::process::kill_process(
            rustix::process::Pid::from_child(&self.child),
            rustix::process::Signal::TERM,
        )
        .expect("向 serve 发 SIGTERM");
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().expect("查询子进程状态").is_none()
    }

    /// 等子进程退出；`GIVE_UP` 到点仍不退就 SIGKILL 收尸并判失败。返回退出状态与耗时。
    async fn exited(&mut self, since: Instant, scenario: &str) -> (ExitStatus, Duration) {
        loop {
            if let Some(status) = self.child.try_wait().expect("查询子进程状态") {
                return (status, since.elapsed());
            }
            if since.elapsed() >= GIVE_UP {
                let _ = self.child.kill();
                let _ = self.child.wait();
                tokio::time::sleep(Duration::from_millis(200)).await;
                panic!(
                    "{scenario}：收到 SIGTERM {} 秒后仍未退出（systemd 会在 20 秒时强杀）。进程输出：\n{}",
                    GIVE_UP.as_secs(),
                    self.output.lock().expect("诊断缓冲")
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn assert_clean(&self, status: ExitStatus, elapsed: Duration, scenario: &str) {
        assert!(
            elapsed <= PROMPT_EXIT,
            "{scenario}：退出用了 {elapsed:?}，超过 {PROMPT_EXIT:?}"
        );
        assert!(
            status.success(),
            "{scenario}：退出状态 {status:?}。进程输出：\n{}",
            self.output.lock().expect("诊断缓冲")
        );
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 只带地址和令牌的 MCP 客户端，可以挪进别的任务。每个请求都走新连接（不留空闲连接）：
/// 关停后的「新会话」必须真的是一条新连接，不能被池里的旧连接冒充。
#[derive(Clone)]
struct Client {
    base: String,
    token: String,
}

impl Client {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .expect("HTTP 客户端")
    }

    fn post(&self, session: Option<&str>, body: Value) -> reqwest::RequestBuilder {
        let mut request = Self::http()
            .post(&self.base)
            .bearer_auth(&self.token)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .timeout(Duration::from_secs(20))
            .body(body.to_string());
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    }

    /// 发一个 JSON-RPC 请求，返回（会话号头，响应对象）；SSE 按 `data:` 事件解析，读到同一 id 即停。
    async fn rpc(
        &self,
        session: Option<&str>,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Option<String>, Value) {
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut response = self
            .post(session, body)
            .send()
            .await
            .expect("发送 JSON-RPC");
        assert_eq!(response.status(), 200, "{method}");
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .map(|value| value.to_str().expect("会话号是 ASCII").to_owned());
        let mut buffer = String::new();
        while let Some(chunk) = response.chunk().await.expect("读取 SSE") {
            buffer.push_str(std::str::from_utf8(&chunk).expect("SSE 是 UTF-8"));
            for event in buffer.split("\n\n") {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim)
                    .collect::<String>();
                if let Ok(value) = serde_json::from_str::<Value>(&data)
                    && value.get("id") == Some(&json!(id))
                {
                    return (session_id, value);
                }
            }
        }
        panic!("{method} 没有收到 JSON-RPC 响应：{buffer}");
    }

    fn initialize_params() -> Value {
        json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "shutdown", "version": "0"}})
    }

    async fn initialize(&self) -> String {
        let (session, response) = self
            .rpc(None, 1, "initialize", Self::initialize_params())
            .await;
        assert!(response.get("result").is_some(), "{response}");
        let session = session.expect("initialize 带会话号");
        let notified = self
            .post(
                Some(&session),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .send()
            .await
            .expect("发送 initialized 通知");
        assert_eq!(notified.status(), 202);
        session
    }

    async fn call(&self, session: &str, id: u64, name: &str, arguments: Value) -> Value {
        let params = json!({"name": name, "arguments": arguments});
        self.rpc(Some(session), id, "tools/call", params).await.1
    }

    /// 开一条 GET 独立 SSE 流（Claude Code 握手后就会开），读到首个事件证明流已建立。
    async fn open_stream(&self, session: &str) -> reqwest::Response {
        let mut stream = Self::http()
            .get(&self.base)
            .bearer_auth(&self.token)
            .header("Accept", "text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Mcp-Session-Id", session)
            .send()
            .await
            .expect("打开 GET SSE 流");
        assert_eq!(stream.status(), 200);
        let first = tokio::time::timeout(Duration::from_secs(5), stream.chunk())
            .await
            .expect("5 秒内收到 SSE 首个事件")
            .expect("读取 SSE");
        assert!(first.is_some(), "SSE 流没有首个事件就结束了");
        stream
    }
}

/// 关停后流必须以正常结束收尾，而不是被掐断或一直挂着。
async fn assert_stream_ended(mut stream: reqwest::Response, scenario: &str) {
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match stream.chunk().await {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    })
    .await;
    match ended {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("{scenario}：SSE 流被掐断而不是正常结束：{error}"),
        Err(_) => panic!("{scenario}：进程退出后 SSE 流仍未结束"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_without_sessions_exits_promptly() {
    let mut agent = Agent::start();
    let since = Instant::now();
    agent.terminate();
    let (status, elapsed) = agent.exited(since, "无会话").await;
    agent.assert_clean(status, elapsed, "无会话");
}

/// 复现 2026-10-15 的生产现场：一个已握手的客户端开着 GET SSE 流，进程收到 SIGTERM 后一直不退。
#[tokio::test(flavor = "multi_thread")]
async fn sigterm_with_an_open_sse_stream_exits_promptly() {
    let mut agent = Agent::start();
    let session = agent.client.initialize().await;
    let stream = agent.client.open_stream(&session).await;
    let since = Instant::now();
    agent.terminate();
    let (status, elapsed) = agent.exited(since, "开着 GET SSE 流").await;
    agent.assert_clean(status, elapsed, "开着 GET SSE 流");
    assert_stream_ended(stream, "开着 GET SSE 流").await;
}

/// 已开始的供给做完才退：工具调用卡在黑板上时收到 SIGTERM，进程等它的结果完整送到客户端，
/// 期间新连接拿不到新会话；放行之后几秒内干净退出，开着的 GET 流正常结束。
#[tokio::test(flavor = "multi_thread")]
async fn sigterm_finishes_the_call_in_progress_and_refuses_new_sessions() {
    let mut agent = Agent::start();
    let session = agent.client.initialize().await;
    let stream = agent.client.open_stream(&session).await;
    let pending = {
        let (client, session) = (agent.client.clone(), session.clone());
        let arguments =
            json!({"project": "项目/Alpha", "operation": "get", "request": {"task_id": "fixture"}});
        tokio::spawn(async move { client.call(&session, 2, "board", arguments).await })
    };
    tokio::time::timeout(Duration::from_secs(10), agent.board.reached.notified())
        .await
        .expect("工具调用 10 秒内到达假黑板");

    agent.terminate();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(agent.running(), "工具调用还卡在半路，进程不能先退出");
    let late = agent
        .client
        .post(
            None,
            json!({"jsonrpc": "2.0", "id": 9, "method": "initialize", "params": Client::initialize_params()}),
        )
        .send()
        .await;
    // 连接被拒（监听已关）或门禁回 503 都算不接新会话；唯独不能拿到 200 和新会话号。
    if let Ok(response) = &late {
        assert_ne!(response.status(), 200, "关停期间不能再建立新会话");
    }

    let released = Instant::now();
    agent.board.release.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("放行后 10 秒内拿到工具结果")
        .expect("调用任务");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(
        response.get("error").is_none()
            && response["result"]["isError"] != json!(true)
            && text.contains(CANARY),
        "卡在半路的那次供给应完整送到客户端：{response}"
    );
    let (status, elapsed) = agent.exited(released, "供给进行中").await;
    agent.assert_clean(status, elapsed, "供给进行中");
    assert_stream_ended(stream, "供给进行中").await;
}
