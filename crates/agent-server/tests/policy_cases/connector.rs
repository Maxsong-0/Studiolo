//! 连接器入口 `/connect/v1/*`（ADR-046）：假权威服务只管答复，看 agent 这一半——权限逐项、状态码、答复头、
//! `ETag`、审计、限频、实例登记、撤权、不可用不冒充别的结论。权威服务那一半在
//! `crates/local-server/tests/board_http/connector.rs`。
use super::{Fixture, NOW, fixture_with_board, is_error, publish};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};
use studiolo_policy::{PolicyKind, RevocationLevel};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const PROJECT: &str = "项目/Alpha";
const OTHER: &str = "项目/Beta";
const BOARD: &str = "homelab-board";
const STATUS: &str = "homelab-status";
const TASK: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const TASK_URL: &str =
    "/connect/v1/board/task?project=%E9%A1%B9%E7%9B%AE%2FAlpha&task_id=01ARZ3NDEKTSV4RRFFQ69G5FAV";

fn capability(kind: &str, board: &[&str], presence: &[&str]) -> String {
    let mut set = json!({"ai_type":kind,
        "read":{"project_whitelist":[],"deny_by_default":true,"rules_whitelist":[]},
        "write":{"board":false,"board_projects":[],"content_patch_only":false,"content_direct":false},
        "policy":{"propose_desired":false,"activate":false},"hard_bound_skills":[]});
    if !board.is_empty() {
        set["read"]["board_projects"] = json!(board);
    }
    if !presence.is_empty() {
        set["write"]["presence_projects"] = json!(presence);
    }
    set.to_string()
}

/// 假权威服务：按设好的答复回，记下收到的请求；可以让它停在半路。
#[derive(Clone)]
struct Authority {
    reply: Arc<Mutex<(u16, Value)>>,
    seen: Arc<Mutex<Vec<Value>>>,
    hold: Arc<Mutex<Option<Arc<tokio::sync::Semaphore>>>>,
    reached: Arc<tokio::sync::Notify>,
}

struct Rig {
    f: Fixture,
    base: String,
    authority: Authority,
    board: String,
    status: String,
    _run: tempfile::TempDir,
}

fn list(projects: &[(&str, &str)]) -> Value {
    json!({"generated_at":jiff_now(),"projects":projects.iter().map(|(project, title)| json!({
        "project":project,"items":[{"id":TASK,"title":title}],"finished":0,"truncated":false
    })).collect::<Vec<_>>()})
}

fn jiff_now() -> String {
    format!("{:?}", std::time::SystemTime::now())
}

fn full(project: &str) -> Value {
    json!({"task":{"task_id":TASK,"spec":{"project":project}},"board":{"x":"BODY_CANARY"}})
}

impl Rig {
    async fn new() -> Self {
        let run = tempfile::TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let socket = run.path().join("b");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let authority = Authority {
            reply: Arc::new(Mutex::new((200, list(&[(PROJECT, "一")])))),
            seen: Arc::default(),
            hold: Arc::default(),
            reached: Arc::default(),
        };
        let server = authority.clone();
        let app = axum::Router::new().route(
            "/internal/connector",
            axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
                let server = server.clone();
                async move {
                    server.seen.lock().unwrap().push(request);
                    let hold = server.hold.lock().unwrap().clone();
                    if let Some(hold) = hold {
                        server.reached.notify_one();
                        let _ = hold.acquire().await.unwrap();
                    }
                    let (status, body) = server.reply.lock().unwrap().clone();
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        axum::Json(body),
                    )
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let f = fixture_with_board(Some(socket)).await;
        let policy = f.policy();
        publish(
            &policy,
            PolicyKind::CapabilitySet,
            "claude",
            Some(&super::capability(&[PROJECT], &[], true)),
        );
        for (kind, board, presence) in [
            (BOARD, &[PROJECT][..], &[][..]),
            (STATUS, &[][..], &[PROJECT][..]),
        ] {
            publish(
                &policy,
                PolicyKind::CapabilitySet,
                kind,
                Some(&capability(kind, board, presence)),
            );
        }
        let board = f.store().issue_type(BOARD, NOW).unwrap().token;
        let status = f.store().issue_type(STATUS, NOW).unwrap().token;
        Self {
            base: f.base.strip_suffix("/mcp").unwrap().to_owned(),
            f,
            authority,
            board,
            status,
            _run: run,
        }
    }

    fn answer(&self, status: u16, body: Value) {
        *self.authority.reply.lock().unwrap() = (status, body);
    }

    fn request(&self, method: &str, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.f
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {token}"))
    }

    async fn get(&self, path: &str, token: &str) -> Reply {
        Reply::read(self.request("GET", path, token)).await
    }

    async fn put(&self, body: Value) -> Reply {
        Reply::read(
            self.request("PUT", "/connect/v1/sessions", &self.status)
                .header("Content-Type", "application/json")
                .body(body.to_string()),
        )
        .await
    }

    fn events(&self, action: &str) -> Vec<Value> {
        self.f
            .ledger()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
            .iter()
            .map(|record| record.event().payload().as_value().clone())
            .collect()
    }

    fn reads(&self) -> Vec<Value> {
        self.events("content.read")
    }

    fn registered(&self) -> usize {
        self.f
            .store()
            .instances(100)
            .unwrap()
            .iter()
            .filter(|(instance, _)| instance.agent_type != "claude")
            .count()
    }

    fn set_agent_store(&self, mode: u32) {
        fs::set_permissions(
            self.f.data.join("agent.sqlite3"),
            fs::Permissions::from_mode(mode),
        )
        .unwrap();
    }
}

struct Reply {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: String,
}

impl Reply {
    /// 每个答复都核：带标记头、不缓存、有正文就是 JSON。
    async fn read(request: reqwest::RequestBuilder) -> Self {
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        assert_eq!(headers["studiolo-connect"], "1");
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        let body = response.text().await.unwrap();
        if status == 304 {
            assert!(body.is_empty());
        } else {
            assert_eq!(headers["content-type"], "application/json");
            serde_json::from_str::<Value>(&body).unwrap();
        }
        Self {
            status,
            headers,
            body,
        }
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }

    fn refused(&self, status: u16, code: &str) {
        assert_eq!(self.status, status, "{}", self.body);
        let body = self.json();
        assert_eq!(body["code"], code);
        assert!(body["error"].is_string());
        assert_eq!(body.as_object().unwrap().len(), 2);
        assert!(!self.body.contains("CANARY"), "{}", self.body);
    }

    fn retry_after(&self) -> u64 {
        self.headers["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap()
    }
}

#[tokio::test]
async fn the_list_is_audited_per_project_and_an_unchanged_one_is_304_without_audit() {
    let r = Rig::new().await;
    publish(
        &r.f.policy(),
        PolicyKind::CapabilitySet,
        BOARD,
        Some(&capability(BOARD, &[PROJECT, OTHER], &[])),
    );
    r.answer(200, list(&[(PROJECT, "一"), (OTHER, "二")]));
    let first = r.get("/connect/v1/board", &r.board).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.json()["projects"][1]["project"], OTHER);
    let tag = first.headers["etag"].to_str().unwrap().to_owned();
    let reads = r.reads();
    assert_eq!(reads.len(), 2);
    for (read, project) in reads.iter().zip([PROJECT, OTHER]) {
        assert_eq!(read["tool"], "connect.board");
        assert_eq!(read["path"], project);
    }
    // 没变（生成时刻变了不算）：304、不加审计；边缘把标签改弱也认。
    for sent in [
        tag.clone(),
        format!("W/{tag}"),
        format!("\"other\", W/{tag}"),
    ] {
        r.answer(200, list(&[(PROJECT, "一"), (OTHER, "二")]));
        let same = Reply::read(
            r.request("GET", "/connect/v1/board", &r.board)
                .header("If-None-Match", sent),
        )
        .await;
        assert_eq!(same.status, 304);
        assert_eq!(same.headers["etag"], tag.as_str());
    }
    assert_eq!(r.reads().len(), 2);
    // 变了：200、新标签、每个项目再各一条。
    r.answer(200, list(&[(PROJECT, "改"), (OTHER, "二")]));
    let changed = Reply::read(
        r.request("GET", "/connect/v1/board", &r.board)
            .header("If-None-Match", &tag),
    )
    .await;
    assert_eq!(changed.status, 200);
    assert_ne!(changed.headers["etag"], tag.as_str());
    assert_eq!(r.reads().len(), 4);
    // 身份由令牌注入，请求里没有别的东西；一个类型一个实例。
    let seen = r.authority.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 5);
    let instance = seen[0]["instance_id"].as_str().unwrap();
    for request in &seen {
        assert_eq!(
            request,
            &json!({"agent_type":BOARD,"instance_id":instance,"issued_at":NOW,
                "operation":"board","request":{}})
        );
    }
    assert_eq!(r.registered(), 1);
    // 权威服务多给了名单外的项目：一项不送。
    publish(
        &r.f.policy(),
        PolicyKind::CapabilitySet,
        BOARD,
        Some(&capability(BOARD, &[PROJECT], &[])),
    );
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(503, "unavailable");
    assert_eq!(r.reads().len(), 4);
}

#[tokio::test]
async fn one_task_is_forwarded_whole_and_outside_or_missing_reads_the_same() {
    let r = Rig::new().await;
    r.answer(200, full(PROJECT));
    let task = r.get(TASK_URL, &r.board).await;
    assert_eq!(task.status, 200);
    assert_eq!(task.json(), full(PROJECT));
    let reads = r.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0]["tool"], "connect.board.task");
    assert_eq!(
        r.authority.seen.lock().unwrap()[0]["request"],
        json!({"task_id":TASK})
    );
    // 名单外的项目不转发；别的项目的任务、不存在，三种同一句话。
    let outside = r.get(&TASK_URL.replace("Alpha", "Beta"), &r.board).await;
    outside.refused(404, "not_found");
    assert_eq!(r.authority.seen.lock().unwrap().len(), 1);
    r.answer(200, full(OTHER));
    let mismatch = r.get(TASK_URL, &r.board).await;
    r.answer(404, json!({"code":"connector_not_found","error":"CANARY"}));
    let missing = r.get(TASK_URL, &r.board).await;
    assert_eq!(
        (outside.status, &outside.body),
        (mismatch.status, &mismatch.body)
    );
    assert_eq!(
        (outside.status, &outside.body),
        (missing.status, &missing.body)
    );
    assert_eq!(r.reads().len(), 1);
    for bad in [
        "/connect/v1/board/task",
        "/connect/v1/board/task?project=x",
        "/connect/v1/board/task?task_id=x",
        "/connect/v1/board/task?project=x&task_id=y&z=1",
        "/connect/v1/board/task?project=x&project=x&task_id=y",
    ] {
        r.get(bad, &r.board).await.refused(400, "invalid_request");
    }
    r.answer(422, json!({"code":"connector_invalid","error":"CANARY"}));
    r.get(TASK_URL, &r.board)
        .await
        .refused(400, "invalid_request");
}

#[tokio::test]
async fn a_snapshot_is_forwarded_untouched_and_every_outcome_has_its_own_status() {
    let r = Rig::new().await;
    let reply = json!({"accepted":1,"rejected":[],"expires_in_secs":300});
    r.answer(200, reply.clone());
    let snapshot = json!({"seq":1,"sessions":[{"kind":"session","anything":"原样转发"}]});
    let accepted = r.put(snapshot.clone()).await;
    assert_eq!((accepted.status, accepted.json()), (200, reply));
    assert_eq!(r.authority.seen.lock().unwrap()[0]["request"], snapshot);
    assert_eq!(r.authority.seen.lock().unwrap()[0]["operation"], "sessions");
    assert!(r.reads().is_empty(), "推状态不在这边记读取");
    for (status, code, outward, outward_code) in [
        (409, "connector_stale", 409, "stale_seq"),
        (422, "connector_invalid", 422, "invalid_sessions"),
        (403, "connector_denied", 403, "not_allowed"),
        (429, "board_busy", 503, "unavailable"),
        (503, "board_unavailable", 503, "unavailable"),
        // 旧的权威服务没有这个入口：不能说成「没有」。
        (404, "", 503, "unavailable"),
        (500, "", 503, "unavailable"),
    ] {
        r.answer(status, json!({"code":code,"error":"CANARY"}));
        let reply = r.put(json!({"seq":2,"sessions":[]})).await;
        reply.refused(outward, outward_code);
        if matches!(outward, 429 | 503) {
            assert!(reply.retry_after() >= 1);
        }
    }
}

#[tokio::test]
async fn malformed_pushes_never_reach_the_authority() {
    let r = Rig::new().await;
    let send = |content_type: &'static str, body: String| {
        Reply::read(
            r.request("PUT", "/connect/v1/sessions", &r.status)
                .header("Content-Type", content_type)
                .body(body),
        )
    };
    for (content_type, body) in [
        ("text/plain", r#"{"seq":1}"#.to_owned()),
        ("application/json", "not json".to_owned()),
        ("application/json", "[]".to_owned()),
        ("application/json", String::new()),
    ] {
        send(content_type, body)
            .await
            .refused(400, "invalid_request");
    }
    send(
        "application/json",
        format!(r#"{{"x":"{}"}}"#, "x".repeat(64 * 1024)),
    )
    .await
    .refused(413, "too_large");
    Reply::read(
        r.request("PUT", "/connect/v1/sessions?x=1", &r.status)
            .header("Content-Type", "application/json")
            .body("{}"),
    )
    .await
    .refused(400, "invalid_request");
    // 看黑板的列表不收任何查询参数。
    r.get("/connect/v1/board?project=x", &r.board)
        .await
        .refused(400, "invalid_request");
    r.get("/connect/v1/sessions", &r.status)
        .await
        .refused(405, "method_not_allowed");
    Reply::read(r.request("POST", "/connect/v1/board", &r.board))
        .await
        .refused(405, "method_not_allowed");
    for path in [
        "/connect",
        "/connect/v1",
        "/connect/v1/boards",
        "/connect/v2/board",
    ] {
        let reply = r.get(path, &r.board).await;
        reply.refused(404, "no_such_entry");
    }
    assert!(r.authority.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn each_token_reaches_only_its_own_entry_and_reads_nothing_anywhere_else() {
    let mut r = Rig::new().await;
    for token in ["", "sag1_wrong"] {
        for path in ["/connect/v1/board", TASK_URL, "/connect/v1/nothing"] {
            let reply = r.get(path, token).await;
            reply.refused(401, "unauthenticated");
            assert!(reply.headers.contains_key("www-authenticate"));
        }
    }
    // 带两份认证头：哪怕有一份是对的也不认。
    Reply::read(
        r.request("GET", "/connect/v1/board", &r.board)
            .header("Authorization", format!("Bearer {}", r.board)),
    )
    .await
    .refused(401, "unauthenticated");
    let claude = r.f.token.clone();
    for token in [&r.status, &claude] {
        for path in ["/connect/v1/board", TASK_URL] {
            r.get(path, token).await.refused(403, "not_allowed");
        }
    }
    for token in [&r.board, &claude] {
        Reply::read(
            r.request("PUT", "/connect/v1/sessions", token)
                .header("Content-Type", "application/json")
                .body(r#"{"seq":1,"sessions":[]}"#),
        )
        .await
        .refused(403, "not_allowed");
    }
    assert_eq!(r.registered(), 0, "没权限的类型不登记实例");
    assert!(r.authority.seen.lock().unwrap().is_empty());
    // 连接器类型走 HTTP 文本入口：403，一项不给。
    for token in [&r.board, &r.status] {
        let reply = r
            .request("GET", "/read/handshake", token)
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 403);
        let body = reply.text().await.unwrap();
        assert!(
            body.starts_with("策略拒绝：") && !body.contains("Alpha"),
            "{body}"
        );
    }
    // 走 MCP：握手只有拒绝原因，之后每个工具都拒。
    for token in [r.board.clone(), r.status.clone()] {
        r.f.token = token;
        let (session, instructions) = r.f.initialize().await.unwrap();
        assert!(instructions.starts_with("策略拒绝："), "{instructions}");
        for leaked in ["# Studiolo 路牌", "Alpha", "入口："] {
            assert!(!instructions.contains(leaked), "{instructions}");
        }
        for (id, (tool, arguments)) in [
            ("handshake", json!({})),
            ("list_entries", json!({"project":PROJECT})),
            ("read_note", json!({"path":"项目/Alpha/00-总览/INDEX.md"})),
            ("search", json!({"project":PROJECT,"text":"Alpha"})),
            ("policy_read", json!({"kind":"rules"})),
            ("policy_read", json!({"kind":"skills"})),
            ("list_proposals", json!({})),
            (
                "propose_desired",
                json!({"kind":"rules","subject":"x.md","body":"x"}),
            ),
            (
                "board",
                json!({"project":PROJECT,"operation":"list","request":{}}),
            ),
            (
                "board",
                json!({"project":PROJECT,"operation":"presence","request":{}}),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let response = r.f.call(&session, id as u64 + 2, tool, arguments).await;
            assert!(is_error(&response), "{tool}: {response}");
            assert!(!response.to_string().contains("Alpha 的事"), "{response}");
        }
    }
    assert!(r.reads().is_empty());
    assert!(r.f.denied_reasons().contains(&"connector_only".to_owned()));
}

#[tokio::test]
async fn content_is_withheld_when_the_grant_or_the_token_goes_away_while_waiting() {
    let r = Arc::new(Rig::new().await);
    type Change = fn(&Rig);
    let cases: [(&str, Change, u16); 3] = [
        (
            "名单里收回这个项目",
            |r: &Rig| {
                publish(
                    &r.f.policy(),
                    PolicyKind::CapabilitySet,
                    BOARD,
                    Some(&capability(BOARD, &[OTHER], &[])),
                );
            },
            503,
        ),
        (
            "令牌轮换",
            |r: &Rig| {
                r.f.store().issue_type(BOARD, NOW).unwrap();
            },
            403,
        ),
        (
            "类型吊销",
            |r: &Rig| {
                r.f.policy()
                    .revoke(RevocationLevel::Type, BOARD, "test", "owner", NOW)
                    .unwrap();
            },
            403,
        ),
    ];
    for path in ["/connect/v1/board", TASK_URL] {
        r.answer(
            200,
            if path == TASK_URL {
                full(PROJECT)
            } else {
                list(&[(PROJECT, "BODY_CANARY")])
            },
        );
        // 先走通一次，免得下面的拒绝是别的原因。
        assert_eq!(r.get(path, &r.board).await.status, 200, "{path}");
    }
    let before = r.reads().len();
    let mut token = r.board.clone();
    for (name, change, expected) in cases {
        publish(
            &r.f.policy(),
            PolicyKind::CapabilitySet,
            BOARD,
            Some(&capability(BOARD, &[PROJECT], &[])),
        );
        r.answer(200, full(PROJECT));
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *r.authority.hold.lock().unwrap() = Some(gate.clone());
        let waiting = {
            let (r, token) = (r.clone(), token.clone());
            tokio::spawn(async move { r.get(TASK_URL, &token).await })
        };
        r.authority.reached.notified().await;
        change(&r);
        gate.add_permits(1);
        *r.authority.hold.lock().unwrap() = None;
        let reply = waiting.await.unwrap();
        assert_eq!(reply.status, expected, "{name}: {}", reply.body);
        assert!(!reply.body.contains("CANARY"), "{name}");
        assert_eq!(r.reads().len(), before, "{name}");
        if name == "令牌轮换" {
            r.get(TASK_URL, &token)
                .await
                .refused(401, "unauthenticated");
            token = r.f.store().issue_type(BOARD, NOW).unwrap().token;
        }
    }
    // 类型吊销之后：照样 403，不是 401。
    r.get(TASK_URL, &token).await.refused(403, "not_allowed");
    // 身份库吊销（撤令牌）：401。
    r.f.store().revoke_type(STATUS, NOW).unwrap();
    r.put(json!({"seq":1,"sessions":[]}))
        .await
        .refused(401, "unauthenticated");
}

#[tokio::test]
async fn instance_revocation_works_and_only_one_instance_is_registered_per_type() {
    let r = Rig::new().await;
    for _ in 0..5 {
        assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    }
    r.answer(200, full(PROJECT));
    assert_eq!(r.get(TASK_URL, &r.board).await.status, 200);
    r.answer(
        200,
        json!({"accepted":0,"rejected":[],"expires_in_secs":300}),
    );
    assert_eq!(r.put(json!({"seq":1,"sessions":[]})).await.status, 200);
    assert_eq!(r.registered(), 2);
    let instances = r.f.store().instances(100).unwrap();
    // 登记审计：没有协商，两格都如实写走的是连接器；整条相等，别的什么都不记。
    let mut recorded = r.events("agent_instance.registered");
    recorded.sort_by_key(|event| event["agent_type"].to_string());
    let expected = [BOARD, STATUS].map(|kind| {
        let (instance, _) = instances
            .iter()
            .find(|(instance, _)| instance.agent_type == kind)
            .unwrap();
        json!({
            "agent_type": kind,
            "agent_instance": instance.id,
            "parent": null,
            "negotiated_version": "not_negotiated:connector",
            "client_reported": "not_negotiated:connector",
        })
    });
    assert_eq!(recorded, expected);
    let (board, last_seen) = instances
        .iter()
        .find(|(instance, _)| instance.agent_type == BOARD)
        .unwrap();
    assert!(
        last_seen > &board.registered_at,
        "接入方页看得到最近一次活动"
    );
    r.f.policy()
        .revoke(RevocationLevel::Instance, &board.id, "test", "owner", NOW)
        .unwrap();
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(403, "not_allowed");
    assert_eq!(r.put(json!({"seq":2,"sessions":[]})).await.status, 200);
    assert_eq!(r.registered(), 2);
}

#[tokio::test]
async fn each_entry_is_limited_and_repeated_denials_are_recorded_once() {
    let r = Rig::new().await;
    for _ in 0..12 {
        assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    }
    let limited = r.get("/connect/v1/board", &r.board).await;
    limited.refused(429, "too_many_requests");
    assert!((1..=60).contains(&limited.retry_after()));
    // 别的入口、别的类型不受影响。
    r.answer(200, full(PROJECT));
    assert_eq!(r.get(TASK_URL, &r.board).await.status, 200);
    for _ in 0..10 {
        r.get("/connect/v1/board", &r.status)
            .await
            .refused(403, "not_allowed");
    }
    let denied = r
        .events("agent.action_denied")
        .into_iter()
        .filter(|event| event["agent_type"] == STATUS)
        .collect::<Vec<_>>();
    assert_eq!(denied.len(), 1, "{denied:?}");
    assert_eq!(denied[0]["reason"], "connector_not_granted");
}

#[tokio::test]
async fn at_most_four_requests_are_in_flight_and_two_of_them_at_the_authority() {
    let r = Arc::new(Rig::new().await);
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *r.authority.hold.lock().unwrap() = Some(gate.clone());
    let mut waiting = Vec::new();
    for index in 0..4 {
        let r = r.clone();
        let reached = r.authority.reached.clone();
        waiting.push(tokio::spawn(async move {
            r.get("/connect/v1/board", &r.board).await.status
        }));
        if index < 2 {
            reached.notified().await;
        }
    }
    // 后两个在这边排队，不去占权威服务那边和别的 AI 共用的名额。
    let queued =
        tokio::time::timeout(Duration::from_millis(500), r.authority.reached.notified()).await;
    assert!(queued.is_err(), "第三个也到了权威服务");
    assert_eq!(r.authority.seen.lock().unwrap().len(), 2);
    let refused = r.get(TASK_URL, &r.board).await;
    refused.refused(429, "too_many_requests");
    assert_eq!(refused.retry_after(), 5);
    *r.authority.hold.lock().unwrap() = None;
    gate.add_permits(4);
    for task in waiting {
        assert_eq!(task.await.unwrap(), 200);
    }
    r.answer(200, full(PROJECT));
    assert_eq!(r.get(TASK_URL, &r.board).await.status, 200);
}

#[tokio::test]
async fn bodies_that_never_finish_hold_no_slot_and_time_out() {
    let r = Rig::new().await;
    let address = r.base.strip_prefix("http://").unwrap().to_owned();
    let mut stuck = Vec::new();
    for _ in 0..4 {
        let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
        let head = format!(
            "PUT /connect/v1/sessions HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\n\
             Content-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\
             Expect: 100-continue\r\n\r\n",
            r.status
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        // 服务端开始读正文才会回这一行：到这里四份都确实在等正文了。
        let mut going = [0; 25];
        stream.read_exact(&mut going).await.unwrap();
        assert_eq!(&going, b"HTTP/1.1 100 Continue\r\n\r\n");
        stream.write_all(b"{\"seq\"").await.unwrap();
        stuck.push(stream);
    }
    // 四份半截正文都在等，别的请求照常进。
    assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    for mut stream in stuck {
        let mut reply = String::new();
        tokio::time::timeout(Duration::from_secs(20), stream.read_to_string(&mut reply))
            .await
            .expect("半截正文没有时限")
            .unwrap();
        assert!(reply.starts_with("HTTP/1.1 408 "), "{reply}");
        assert!(reply.contains("studiolo-connect: 1"), "{reply}");
        assert!(reply.contains("\"code\":\"body_timeout\""), "{reply}");
    }
    assert_eq!(r.registered(), 1, "半截的推送没登记实例");
}

#[tokio::test]
async fn unrecognized_callers_are_recorded_once_a_minute_not_once_a_request() {
    let r = Rig::new().await;
    for token in ["", "sag1_wrong"] {
        for _ in 0..10 {
            r.get("/connect/v1/board", token)
                .await
                .refused(401, "unauthenticated");
        }
    }
    let anonymous = |r: &Rig| {
        r.events("agent.action_denied")
            .into_iter()
            .filter(|event| event["action"] == "connect.request")
            .map(|event| event["reason"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(anonymous(&r), ["unauthenticated"]);
    // 身份库读不出来是另一个原因，也只记一条。
    r.set_agent_store(0o640);
    for _ in 0..5 {
        r.get("/connect/v1/board", &r.board)
            .await
            .refused(503, "unavailable");
    }
    r.set_agent_store(0o600);
    assert_eq!(anonymous(&r), ["unauthenticated", "identity_unavailable"]);
}

#[tokio::test]
async fn a_frozen_ledger_lets_nothing_into_the_identity_store() {
    let r = Rig::new().await;
    let freeze = || {
        r.f.ledger()
            .quiesce_operations(Instant::now() + Duration::from_secs(2))
            .unwrap()
    };
    let frozen = freeze();
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(503, "unavailable");
    assert_eq!(r.registered(), 0, "冻结中登记了实例");
    drop(frozen);
    assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    let last_seen = |r: &Rig| {
        r.f.store()
            .instances(100)
            .unwrap()
            .into_iter()
            .find(|(instance, _)| instance.agent_type == BOARD)
            .unwrap()
            .1
    };
    let before = last_seen(&r);
    let reached = r.authority.seen.lock().unwrap().len();
    let frozen = freeze();
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(503, "unavailable");
    assert_eq!(last_seen(&r), before, "冻结中改了最近活动");
    assert_eq!(r.authority.seen.lock().unwrap().len(), reached);
    drop(frozen);
}

#[tokio::test]
async fn failing_to_record_a_registration_never_registers_a_second_instance() {
    let r = Rig::new().await;
    // 账本版本对不上：许可取得到，写不进去。
    let ledger = rusqlite::Connection::open(r.f.data.join("audit/audit.sqlite3")).unwrap();
    let version: i64 = ledger
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    ledger.execute_batch("PRAGMA user_version=999").unwrap();
    for _ in 0..5 {
        r.get("/connect/v1/board", &r.board)
            .await
            .refused(503, "unavailable");
    }
    assert!(r.authority.seen.lock().unwrap().is_empty());
    assert_eq!(r.registered(), 1);
    ledger
        .execute_batch(&format!("PRAGMA user_version={version}"))
        .unwrap();
    drop(ledger);
    for _ in 0..2 {
        assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    }
    assert_eq!(r.registered(), 1);
    let registered = r
        .events("agent_instance.registered")
        .into_iter()
        .filter(|event| event["agent_type"] == BOARD)
        .count();
    assert_eq!(registered, 1, "补登记的审计恰好一条");
}

#[tokio::test]
async fn an_unreadable_store_or_ledger_is_unavailable_never_a_denial_or_an_empty_board() {
    let r = Rig::new().await;
    // 身份库读不出来：503、不带 challenge，不冒充令牌错。
    r.set_agent_store(0o640);
    let reply = r.get("/connect/v1/board", &r.board).await;
    reply.refused(503, "unavailable");
    assert!(!reply.headers.contains_key("www-authenticate"));
    r.set_agent_store(0o600);
    assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    let reads = r.reads().len();
    // 账本冻结：登记不了就一项不给。
    let ledger = r.f.ledger();
    let frozen = ledger
        .quiesce_operations(Instant::now() + Duration::from_secs(2))
        .unwrap();
    r.answer(200, list(&[(PROJECT, "BODY_CANARY")]));
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(503, "unavailable");
    drop(frozen);
    // 账本目录权限不对：同样一项不给。
    let audit = r.f.data.join("audit");
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o750)).unwrap();
    r.get("/connect/v1/board", &r.board)
        .await
        .refused(503, "unavailable");
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(r.get("/connect/v1/board", &r.board).await.status, 200);
    assert_eq!(r.reads().len(), reads + 1);
}
