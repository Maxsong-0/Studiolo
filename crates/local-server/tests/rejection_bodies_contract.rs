//! 拒绝与空正文契约：黑板、执行控制与策略版本路由的提取器拒绝，以及原先空正文的
//! 404／403／503，一律答复固定中文 JSON Problem；状态码不变，正文不回显字段名、原始取值
//! 或解析器英文细节。黑板平面照 board_http_contract.rs 装配；所有数据均为临时虚构夹具。
#[allow(dead_code)]
#[path = "writer_http/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

use axum::{body::to_bytes, http::header::CONTENT_TYPE, response::Response};
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_local_server::{
    PreparedUnixIngress,
    board::{BoardPlane, private_board_router, with_board_plane},
};
use studiolo_policy::PolicyStore;
use studiolo_supply::AgentStore;

/// 查询串里的 `项目/Studiolo`。
const PROJECT: &str = "%E9%A1%B9%E7%9B%AE%2FStudiolo";
const TASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const RANGE: &str = "from=2026-09-01T00:00:00Z&to=2026-09-02T00:00:00Z";

/// 被拒绝的字段名与解析器英文细节；任何一个出现在正文里都算回显。
/// 前 12 项照抄 error_responses_contract.rs，其余是黑板请求字段与 axum 默认拒绝的措辞。
const LEAKS: [&str; 19] = [
    "generation",
    "path",
    "mode",
    "offset",
    "refresh",
    "project",
    "root",
    "missing",
    "unknown",
    "field",
    "deserialize",
    "Failed",
    "task_id",
    "agent_type",
    "instance_id",
    "operation",
    "Expected",
    "Invalid",
    "Cannot",
];

/// 固定中文 Problem：状态、内容类型与正文逐字相符，且不含任何 LEAKS。
fn assert_problem(reply: (u16, String, String), status: u16, code: &str, error: &str) {
    let (got, content_type, body) = reply;
    assert_eq!(got, status, "{code}：{body}");
    assert_eq!(content_type, "application/json", "{code}：{body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value, json!({"code": code, "error": error}), "{body}");
    for leaked in LEAKS {
        assert!(!body.contains(leaked), "{code} 回显了 {leaked}：{body}");
    }
}

async fn parts(response: Response) -> (u16, String, String) {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (
        status,
        content_type,
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn routes_without_a_plane_or_with_a_bad_version_answer_fixed_chinese_json() {
    let f = support::Fixture::new(true).await;
    let login = f.issue().await;
    let get = |uri: &str| f.request("GET", uri, Some(&login.cookie), "");
    // 查询串合法而黑板平面未装：三条监督路由仍是 404，正文从空换成固定中文 JSON。
    for uri in [
        format!("/api/v1/board?project={PROJECT}"),
        format!("/api/v1/board/replay?task_id={TASK}&after=0"),
        format!("/api/v1/board/timeline?project={PROJECT}&{RANGE}"),
    ] {
        let reply = parts(f.send(get(&uri), 404).await).await;
        assert_problem(reply, 404, "board_unconfigured", "黑板未配置。");
    }
    // 执行控制：会话、CSRF 与正文都合法，只是没装黑板平面。
    let mut command = f.request(
        "POST",
        "/api/v1/execution/command",
        Some(&login.cookie),
        r#"{"operation":"freeze","frozen":true}"#,
    );
    command
        .headers_mut()
        .insert("x-csrf-token", login.csrf.parse().unwrap());
    let reply = parts(f.send(command, 404).await).await;
    assert_problem(reply, 404, "execution_unconfigured", "执行控制未配置。");
    // 版本号不是 u64（含溢出）：拒绝先于平面检查，仍是 400，也不回显原始路径段。
    for raw in ["not-a-version", "18446744073709551616"] {
        let uri = format!("/api/v1/policy/versions/{raw}");
        let reply = parts(f.send(get(&uri), 400).await).await;
        assert!(!reply.2.contains(raw), "回显了 {raw}：{}", reply.2);
        assert_problem(reply, 400, "invalid_policy_version", "策略版本号不合法。");
    }
    // 版本号合法而策略平面未装：仍是原有的 404 policy_plane_absent，不因拒绝改造而变。
    let reply = parts(f.send(get("/api/v1/policy/versions/1"), 404).await).await;
    assert_problem(reply, 404, "policy_plane_absent", "本入口未装配策略平面。");
}

/// 真实写者、独立 0700 控制目录、身份库与策略库，以及只听同 UID 的私有 Unix 入口。
struct Board {
    writer: fixture::Fixture,
    client: reqwest::Client,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Board {
    async fn new() -> Self {
        let mut writer = fixture::Fixture::editable().await;
        let root = writer.http.root.clone();
        for dir in ["board", "agent", "policy", "board-run"] {
            fs::create_dir(root.join(dir)).unwrap();
            support::mode(&root.join(dir), 0o700);
        }
        drop(AgentStore::open(&root.join("agent")).unwrap());
        drop(PolicyStore::open(&root.join("policy")).unwrap());
        let audit = Arc::new(SqliteAuditLedger::open_existing(&writer.http.audit).unwrap());
        let dirs = [
            root.join("vault"),
            root.join("web"),
            root.join("agent"),
            root.join("policy"),
            writer.data.clone(),
        ];
        let isolated = dirs.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        let plane = BoardPlane::open(
            writer.owner.clone(),
            &root.join("board"),
            &root.join("agent"),
            &root.join("policy"),
            &isolated,
            audit,
        )
        .unwrap();
        let socket = root.join("board-run/b");
        let bound = PreparedUnixIngress::prepare(&socket, &isolated)
            .unwrap()
            .bind()
            .unwrap();
        let app = private_board_router(plane.clone());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            bound
                .serve_shutdown(app, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap()
        });
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .no_proxy()
            .build()
            .unwrap();
        writer.http.app = with_board_plane(writer.http.app, plane);
        Self {
            writer,
            client,
            stop,
            server,
        }
    }

    /// 经真实 Unix 入口投递一份原始正文；`source` 决定是否带唯一来源头。
    async fn post(
        &self,
        content_type: Option<&str>,
        source: bool,
        body: Vec<u8>,
    ) -> (u16, String, String) {
        let mut request = self
            .client
            .post("http://localhost/internal/board")
            .body(body);
        if source {
            request = request.header("cf-connecting-ip", "127.0.0.1");
        }
        if let Some(value) = content_type {
            request = request.header("content-type", value);
        }
        let reply = request.send().await.unwrap();
        let status = reply.status().as_u16();
        let content_type = reply
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = String::from_utf8(reply.bytes().await.unwrap().to_vec()).unwrap();
        (status, content_type, body)
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.server.await.unwrap();
        self.writer.close().await;
    }
}

#[tokio::test]
async fn board_rejections_and_failures_keep_status_and_answer_fixed_chinese_json() {
    let f = Board::new().await;
    let json_type = Some("application/json");
    let valid = json!({"agent_type": "claude", "instance_id": "fixture",
        "project": "项目/Studiolo", "operation": "list", "request": {}});
    let (mut smuggled, mut escaped) = (valid.clone(), valid.clone());
    smuggled["smuggled"] = json!(1);
    escaped["project"] = json!("../escape");
    // 私有入口：真实同 UID 连接但缺唯一来源头，仍不受信，先于正文解析答 403。
    let reply = f.post(json_type, false, valid.to_string().into()).await;
    assert_problem(reply, 403, "board_peer_forbidden", "只接受本机受信连接。");
    // 正文拒绝沿用原提取器状态码：语法错 400、缺 JSON 类型 415、未知字段与非法取值 422、
    // 超过 256 KiB 上限 413（恰多一字节，服务端读完整份正文才判超长）。
    for (content_type, body, status, echo) in [
        (json_type, br#"{"agent_type":"#.to_vec(), 400, "EOF"),
        (None, valid.to_string().into(), 415, "application/json"),
        (json_type, smuggled.to_string().into(), 422, "smuggled"),
        (json_type, escaped.to_string().into(), 422, "escape"),
        (json_type, vec![b' '; 262_145], 413, "limit"),
    ] {
        let reply = f.post(content_type, true, body).await;
        assert!(
            !reply.2.contains(echo),
            "{status} 回显了 {echo}：{}",
            reply.2
        );
        assert_problem(reply, status, "invalid_board_request", "黑板请求不合法。");
    }
    // 人侧监督路由：控制目录权限一变，存储复核即失败，三条路由都答 503 固定中文 JSON。
    let http = &f.writer.http;
    let login = http.issue().await;
    let control = http.root.join("board");
    support::mode(&control, 0o750);
    for uri in [
        format!("/api/v1/board?project={PROJECT}"),
        format!("/api/v1/board/replay?task_id={TASK}"),
        format!("/api/v1/board/timeline?project={PROJECT}&{RANGE}"),
    ] {
        let request = http.request("GET", &uri, Some(&login.cookie), "");
        let reply = parts(http.send(request, 503).await).await;
        assert_problem(reply, 503, "board_unavailable", "黑板暂不可用，请重试。");
    }
    // 权限复原后同一平面照常答复：上面的 503 只来自那次存储复核失败。
    support::mode(&control, 0o700);
    let uri = format!("/api/v1/board?project={PROJECT}");
    assert_eq!(f.writer.get(&uri, &login.cookie, 200).await, json!([]));
    f.stop().await;
}
