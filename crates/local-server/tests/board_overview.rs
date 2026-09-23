//! T-130 跨项目只读汇总 `GET /api/v1/board/overview`：真实会话门禁 + 真实 Unix 黑板入口造数；
//! 无生产数据。时间窗用「把已写记录整体挪到三天前」的账本装饰器模拟，截断用真实写入的足量数据。
#[allow(dead_code)]
#[path = "writer_http/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    AuditSubject, MemoryAuditLedger, SanitizedPayload, SqliteAuditLedger,
};
use studiolo_control_db::ControlDb;
use studiolo_core::{TaskId, VaultUuid};
use studiolo_local_server::{
    PreparedUnixIngress,
    board::{BoardPlane, private_board_router, with_board_plane},
};
use studiolo_policy::{DesiredInput, DesiredStatus, PolicyKind, PolicyStore, Submitter};
use studiolo_supply::AgentStore;
use studiolo_task_engine::{BoardTaskSpec, PersistentTaskEngine};
use studiolo_vault_broker::MemoryVaultBroker;

const PROJECT: &str = "项目/Studiolo";
const OTHER: &str = "项目/Other";
const NOW: &str = "2026-09-20T00:00:00Z";
const OVERVIEW: &str = "/api/v1/board/overview";
const CRITERION: &str = "汇总已核对";
const INSTRUCTION: &str = "不进汇总的指令正文";

/// 把 offset 不超过 `aged_through` 的记录整体挪到三天前，模拟「这些事件早已发生」；
/// 追加、幂等、过滤、顺序与偏移全部委托给真实账本，只改读出记录的时间戳。
struct AgedLedger {
    inner: Arc<dyn AuditLedger>,
    aged_through: AtomicU64,
}

impl AgedLedger {
    fn age_everything_so_far(&self) {
        let last = self
            .inner
            .query(&AuditFilter::default())
            .unwrap()
            .last()
            .map_or(0, AuditRecord::offset);
        self.aged_through.store(last, Ordering::SeqCst);
    }
}

impl AuditLedger for AgedLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let cutoff = self.aged_through.load(Ordering::SeqCst);
        Ok(self
            .inner
            .query(filter)?
            .into_iter()
            .map(|record| {
                if record.offset() > cutoff {
                    return record;
                }
                AuditRecord::from_parts(
                    record.offset(),
                    record
                        .recorded_at()
                        .checked_sub(SignedDuration::from_hours(72))
                        .unwrap(),
                    record.previous_hash(),
                    record.record_hash(),
                    record.event().clone(),
                )
            })
            .collect())
    }
}

enum Ledger {
    /// 与生产同一份 SQLite 审计库，外加时间装饰器。
    Aged,
    /// 内存账本：只用于需要上千条真实追加的截断场景。
    Memory,
}

fn publish(policy: &PolicyStore, kind: &str) {
    let body = json!({"ai_type":kind,
        "read":{"project_whitelist":[PROJECT,OTHER],"deny_by_default":true},
        "write":{"board":true,"board_projects":[PROJECT,OTHER],"content_patch_only":false,
            "content_direct":false},
        "policy":{"propose_desired":true,"activate":false},"hard_bound_skills":[]})
    .to_string();
    let desired = policy
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: kind.into(),
                body: Some(body),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(
        desired.status,
        DesiredStatus::Validated,
        "{:?}",
        desired.report
    );
    policy
        .activate(
            &desired.id,
            &policy.effective().unwrap().effective_hash,
            "fixture-owner",
            NOW,
        )
        .unwrap();
}

fn spec(project: &str, adapter: &str, title: Option<&str>, path: &str) -> Value {
    json!({"task":{"role":"author","adapter":adapter,"input_paths":[],"allow_web":[],"allow_db":[],
        "retry_policy":{"max_attempts":1,"backoff_seconds":0,"retry_on":[]},"timeout":300,
        "project":project,"parent":null,"title":title,"instruction":INSTRUCTION},
        "acceptance_criteria":[CRITERION],"files_owned":[format!("{project}/{path}")],
        "max_children":8,"artifact_target":null})
}

fn completed() -> Value {
    json!({"status":"completed","artifacts":[],"conflicts":[],"unlanded":[],
        "acceptance":[{"criterion":CRITERION,"result":"pass","evidence":[]}],
        "remaining":[],"elapsed_ms":100,"tool_calls":1,"learned":[],
        "closeout":{"learned":[],"adr_required":false,"rule_change_required":false,"destinations":[]}})
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("应为对象：{value}"))
        .keys()
        .map(String::as_str)
        .collect()
}

fn set<'a>(names: &[&'a str]) -> BTreeSet<&'a str> {
    names.iter().copied().collect()
}

fn time(value: &Value) -> Timestamp {
    let text = value.as_str().unwrap();
    assert!(text.ends_with('Z'), "应为 RFC3339 UTC：{text}");
    text.parse().unwrap()
}

fn find<'a>(overview: &'a Value, id: &Value) -> Option<&'a Value> {
    overview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| &item["id"] == id)
}

struct Harness {
    writer: fixture::Fixture,
    identities: AgentStore,
    policy: PolicyStore,
    client: reqwest::Client,
    instance: String,
    aged: Option<Arc<AgedLedger>>,
    memory: Option<Arc<MemoryAuditLedger>>,
    cookie: String,
    plane: Arc<BoardPlane>,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn new(ledger: Ledger) -> Self {
        Self::build(ledger, |_, _| ()).await.0
    }

    /// `seed` 在黑板平面打开前拿到控制目录与 Vault 身份，可直接经真实引擎预置工作项。
    async fn build<T>(ledger: Ledger, seed: impl FnOnce(&Path, VaultUuid) -> T) -> (Self, T) {
        let mut writer = fixture::Fixture::editable().await;
        let root = writer.http.root.clone();
        for dir in ["board", "agent", "policy", "board-run"] {
            fs::create_dir(root.join(dir)).unwrap();
            support::mode(&root.join(dir), 0o700);
        }
        for project in [PROJECT, OTHER] {
            fs::create_dir_all(root.join("vault").join(project).join("04-协调")).unwrap();
            fs::write(
                root.join("vault").join(project).join("04-协调/TASKS.md"),
                "# 人工协调\n",
            )
            .unwrap();
        }
        let identities = AgentStore::open(&root.join("agent")).unwrap();
        identities.upgrade_child_instances().unwrap();
        identities.issue_type("claude", NOW).unwrap();
        let instance = identities
            .register_instance("claude", "overview-parent", NOW)
            .unwrap()
            .id;
        let policy = PolicyStore::open(&root.join("policy")).unwrap();
        publish(&policy, "claude");
        let seeded = seed(&root.join("board"), writer.expected.vault_uuid);
        let (aged, memory, audit): (_, _, Arc<dyn AuditLedger>) = match ledger {
            Ledger::Aged => {
                let aged = Arc::new(AgedLedger {
                    inner: Arc::new(SqliteAuditLedger::open_existing(&writer.http.audit).unwrap()),
                    aged_through: AtomicU64::new(0),
                });
                (Some(aged.clone()), None, aged)
            }
            Ledger::Memory => {
                let memory = Arc::new(MemoryAuditLedger::new());
                (None, Some(memory.clone()), memory)
            }
        };
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
        let (stop, rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            bound
                .serve_shutdown(app, async {
                    let _ = rx.await;
                })
                .await
                .unwrap()
        });
        let client = reqwest::Client::builder()
            .unix_socket(socket)
            .no_proxy()
            .build()
            .unwrap();
        writer.http.app = with_board_plane(writer.http.app, plane.clone());
        let cookie = writer.http.issue().await.cookie;
        (
            Self {
                writer,
                identities,
                policy,
                client,
                instance,
                aged,
                memory,
                cookie,
                plane,
                stop,
                server,
            },
            seeded,
        )
    }

    async fn call_as(&self, instance: &str, project: &str, op: &str, request: Value) -> Value {
        let reply = self
            .client
            .post("http://localhost/internal/board")
            .header("cf-connecting-ip", "127.0.0.1")
            .header("content-type", "application/json")
            .body(
                json!({"agent_type":"claude","instance_id":instance,"project":project,
                    "operation":op,"request":request})
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        let status = reply.status().as_u16();
        let body: Value = serde_json::from_slice(&reply.bytes().await.unwrap()).unwrap();
        assert_eq!(status, 200, "{op}: {body}");
        body
    }

    async fn create(&self, project: &str, key: &str, adapter: &str, title: Option<&str>) -> Value {
        self.call_as(
            &self.instance,
            project,
            "create",
            json!({"request_id":key,"spec":spec(project, adapter, title, key)}),
        )
        .await
    }

    /// 以指定实例认领，返回后续写操作要带的 identity。
    async fn claim(&self, instance: &str, project: &str, task: &Value) -> Value {
        let lease = Timestamp::now().as_millisecond() + 60_000;
        let claim = self
            .call_as(
                instance,
                project,
                "claim",
                json!({"task_id":task,"lease_until_ms":lease}),
            )
            .await;
        json!({"task_id":task,"attempt_id":claim["attempt"]["attempt_id"],
            "fence":claim["attempt"]["fence"]})
    }

    async fn heartbeat(&self, identity: &Value, tools: u64, edits: u64) {
        let lease = Timestamp::now().as_millisecond() + 60_000;
        self.call_as(
            &self.instance,
            PROJECT,
            "heartbeat",
            json!({"identity":identity,"lease_until_ms":lease,"tool_calls":tools,"edits":edits}),
        )
        .await;
    }

    async fn complete(&self, identity: &Value) {
        self.call_as(
            &self.instance,
            PROJECT,
            "return",
            json!({"identity":identity,"receipt":completed()}),
        )
        .await;
    }

    async fn overview(&self, query: &str) -> Value {
        self.writer
            .get(&format!("{OVERVIEW}{query}"), &self.cookie, 200)
            .await
    }

    async fn status(&self, query: &str, expected: u16) {
        let http = &self.writer.http;
        let response = http
            .send(
                http.request("GET", &format!("{OVERVIEW}{query}"), Some(&self.cookie), ""),
                expected,
            )
            .await;
        if expected == 400 {
            invalid_query(response, query).await;
        }
    }

    async fn stop(self) {
        let Self {
            stop,
            server,
            writer,
            plane,
            ..
        } = self;
        stop.send(()).unwrap();
        server.await.unwrap();
        drop(plane);
        writer.owner.shutdown().await.unwrap();
    }
}

/// 每条事件都落在窗口内、按时间升序，且其工作项与项目都出现在 items 里。
fn assert_events_consistent(overview: &Value) {
    let from = time(&overview["window_from"]);
    let to = time(&overview["window_to"]);
    let mut previous = from;
    for event in overview["events"].as_array().unwrap() {
        let at = time(&event["at"]);
        assert!(from <= at && at <= to, "事件落在窗口外：{event}");
        assert!(previous <= at, "事件必须按时间升序：{event}");
        previous = at;
        let item = find(overview, &event["task_id"])
            .unwrap_or_else(|| panic!("事件所属工作项不在 items 里：{event}"));
        assert_eq!(event["project"], item["project"]);
        assert!(matches!(
            event["phase"].as_str(),
            Some("intent" | "outcome")
        ));
        assert!(!event["action"].as_str().unwrap().is_empty());
        assert!(
            event["attempt_id"].is_null()
                || event["attempt_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("A-"))
        );
    }
}

#[tokio::test]
async fn overview_is_session_gated_and_absent_without_a_board_plane() {
    let writer = fixture::Fixture::editable().await;
    let http = &writer.http;
    http.send(http.request("GET", OVERVIEW, None, ""), 401)
        .await;
    let login = http.issue().await;
    // 能解析的查询（含越界值）先判装配，解析失败的查询串先得 400。
    for (query, status) in [
        ("", 404),
        ("?window_hours=24", 404),
        ("?window_hours=0", 404),
        ("?window_hours=abc", 400),
        ("?project=x", 400),
    ] {
        let response = http
            .send(
                http.request(
                    "GET",
                    &format!("{OVERVIEW}{query}"),
                    Some(&login.cookie),
                    "",
                ),
                status,
            )
            .await;
        if status == 400 {
            invalid_query(response, query).await;
        }
    }
    writer.close().await;
}

#[tokio::test]
async fn overview_rejects_malformed_windows_and_unknown_parameters() {
    let h = Harness::new(Ledger::Aged).await;
    let http = &h.writer.http;
    http.send(http.request("GET", OVERVIEW, None, ""), 401)
        .await;
    for query in [
        "?window_hours=0",
        "?window_hours=169",
        "?window_hours=abc",
        "?window_hours=1.5",
        "?window_hours=-1",
        "?window_hours=",
        "?window_hours=4294967296",
        "?window_hours=24&window_hours=24",
        "?window_hours=24&project=%E9%A1%B9%E7%9B%AE%2FStudiolo",
        "?project=%E9%A1%B9%E7%9B%AE%2FStudiolo",
        "?WINDOW_HOURS=24",
    ] {
        h.status(query, 400).await;
    }
    for (query, hours) in [("", 24), ("?window_hours=1", 1), ("?window_hours=168", 168)] {
        let overview = h.overview(query).await;
        let span = time(&overview["window_to"]).duration_since(time(&overview["window_from"]));
        assert_eq!(span, SignedDuration::from_hours(hours), "{query}");
    }
    h.stop().await;
}

#[tokio::test]
async fn overview_fields_are_closed_and_match_the_board_view() {
    let h = Harness::new(Ledger::Aged).await;
    let done = h.create(PROJECT, "done", "consumer", Some("已完成")).await;
    let identity = h.claim(&h.instance, PROJECT, &done).await;
    h.complete(&identity).await;
    let running = h
        .create(PROJECT, "running", "consumer", Some("进行中"))
        .await;
    let identity = h.claim(&h.instance, PROJECT, &running).await;
    h.heartbeat(&identity, 3, 1).await;
    let draft = h.create(OTHER, "draft", "codex", None).await;
    let before = Timestamp::now();
    let overview = h.overview("").await;
    let after = Timestamp::now();

    assert_eq!(
        keys(&overview),
        set(&[
            "generated_at",
            "window_from",
            "window_to",
            "policy_hash",
            "items",
            "items_truncated",
            "events",
            "events_truncated",
        ])
    );
    let item_keys = set(&[
        "id",
        "title",
        "project",
        "status",
        "parent",
        "adapter",
        "holder",
        "holder_lineage",
        "paths",
        "attempt",
        "progress",
        "receipt_status",
    ]);
    let items = overview["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for item in items {
        assert_eq!(keys(item), item_keys, "{item}");
        for node in item["holder_lineage"].as_array().unwrap() {
            assert_eq!(keys(node), set(&["id", "agent_type"]));
        }
        if !item["attempt"].is_null() {
            assert_eq!(
                keys(&item["attempt"]),
                set(&["phase", "worker", "active", "lease_until_ms"])
            );
        }
        if !item["progress"].is_null() {
            assert_eq!(
                keys(&item["progress"]),
                set(&["tools", "edits", "last_progress_ms"])
            );
        }
    }
    let events = overview["events"].as_array().unwrap();
    assert!(!events.is_empty());
    for event in events {
        assert_eq!(
            keys(event),
            set(&["at", "project", "task_id", "action", "phase", "attempt_id"])
        );
    }
    assert!(!overview.to_string().contains(INSTRUCTION));
    assert!(!overview.to_string().contains(CRITERION));

    // 时间：RFC3339 UTC，生成时刻就是窗口右端，默认 24 小时。
    let generated = time(&overview["generated_at"]);
    assert_eq!(overview["generated_at"], overview["window_to"]);
    assert!(before <= generated && generated <= after);
    assert_eq!(
        time(&overview["window_to"]).duration_since(time(&overview["window_from"])),
        SignedDuration::from_hours(24)
    );
    assert_eq!(
        overview["policy_hash"],
        h.policy.effective().unwrap().effective_hash
    );
    assert_eq!(overview["items_truncated"], false);
    assert_eq!(overview["events_truncated"], false);
    assert_events_consistent(&overview);

    let running_item = find(&overview, &running).unwrap();
    assert_eq!(running_item["status"], "running");
    assert_eq!(running_item["title"], "进行中");
    assert_eq!(running_item["adapter"], "consumer");
    assert_eq!(running_item["holder"], h.instance.as_str());
    assert_eq!(running_item["paths"], json!([format!("{PROJECT}/running")]));
    assert_eq!(running_item["attempt"]["phase"], "running");
    assert_eq!(running_item["attempt"]["worker"], h.instance.as_str());
    assert_eq!(running_item["attempt"]["active"], true);
    assert!(running_item["attempt"]["lease_until_ms"].as_i64().unwrap() > 0);
    assert_eq!(running_item["progress"]["tools"], 3);
    assert_eq!(running_item["progress"]["edits"], 1);
    assert!(
        running_item["progress"]["last_progress_ms"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert_eq!(running_item["receipt_status"], Value::Null);

    let done_item = find(&overview, &done).unwrap();
    assert_eq!(done_item["status"], "succeeded");
    assert_eq!(done_item["receipt_status"], "completed");
    assert_eq!(done_item["attempt"]["phase"], "completed");
    assert_eq!(done_item["attempt"]["active"], false);

    let draft_item = find(&overview, &draft).unwrap();
    assert_eq!(draft_item["project"], OTHER);
    assert_eq!(draft_item["adapter"], "codex");
    assert_eq!(draft_item["title"], draft, "缺标题时回落任务 id");
    assert_eq!(draft_item["holder"], Value::Null);
    assert_eq!(draft_item["holder_lineage"], json!([]));
    assert_eq!(draft_item["attempt"], Value::Null);
    assert_eq!(draft_item["progress"], Value::Null);
    assert_eq!(draft_item["parent"], Value::Null);

    // holder_lineage 与 /api/v1/board 里 holder 那条实例链同源（根→叶）。
    let board = h
        .writer
        .get(
            "/api/v1/board?project=%E9%A1%B9%E7%9B%AE%2FStudiolo",
            &h.cookie,
            200,
        )
        .await;
    for task in [&running, &done] {
        let view = board
            .as_array()
            .unwrap()
            .iter()
            .find(|view| &view["task"]["task_id"] == task)
            .unwrap();
        let chain = view["instance_lineages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|chain| chain.as_array().unwrap().last().unwrap()["id"] == view["holder"])
            .unwrap();
        let item = find(&overview, task).unwrap();
        assert_eq!(&item["holder_lineage"], chain);
        assert_eq!(item["holder"], view["holder"]);
        assert_eq!(item["status"], view["task"]["status"]);
    }
    h.stop().await;
}

#[tokio::test]
async fn terminal_items_need_window_events_and_events_ascend() {
    let h = Harness::new(Ledger::Aged).await;
    // 先发生、随后被整体挪到三天前的一批：一个已完成、一个仍在进行。
    let stale_done = h
        .create(PROJECT, "stale-done", "consumer", Some("三天前已完成"))
        .await;
    let identity = h.claim(&h.instance, PROJECT, &stale_done).await;
    h.complete(&identity).await;
    let stale_running = h
        .create(PROJECT, "stale-running", "consumer", Some("三天前开始"))
        .await;
    let identity = h.claim(&h.instance, PROJECT, &stale_running).await;
    h.heartbeat(&identity, 2, 0).await;
    h.aged.as_ref().unwrap().age_everything_so_far();

    // 窗口内新发生的一批：一个刚完成、一个别的项目的草稿、一个由子实例认领。
    let fresh_done = h
        .create(PROJECT, "fresh-done", "consumer", Some("刚完成"))
        .await;
    let identity = h.claim(&h.instance, PROJECT, &fresh_done).await;
    h.complete(&identity).await;
    let other_draft = h
        .create(OTHER, "other-draft", "consumer", Some("别的项目"))
        .await;
    let child = h
        .identities
        .register_child_instance("claude", &h.instance, "overview-child", NOW)
        .unwrap();
    let delegated = h
        .create(PROJECT, "delegated", "consumer", Some("子实例在做"))
        .await;
    h.claim(&child.id, PROJECT, &delegated).await;

    let day = h.overview("?window_hours=24").await;
    let ids: Vec<&Value> = day["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| &item["id"])
        .collect();
    let mut studiolo = vec![&fresh_done, &stale_running, &delegated];
    studiolo.sort_by_key(|id| id.as_str().unwrap().to_owned());
    let expected: Vec<&Value> = [vec![&other_draft], studiolo].concat();
    assert_eq!(ids, expected, "窗口外已结束项被排除；按 (项目, id) 排序");
    assert_events_consistent(&day);
    let with_events: BTreeSet<&str> = day["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["task_id"].as_str().unwrap())
        .collect();
    for task in [&fresh_done, &other_draft, &delegated] {
        assert!(with_events.contains(task.as_str().unwrap()), "{task}");
    }
    for task in [&stale_done, &stale_running] {
        assert!(!with_events.contains(task.as_str().unwrap()), "{task}");
    }
    let running = find(&day, &stale_running).unwrap();
    assert_eq!(running["attempt"]["active"], true);
    assert_eq!(running["progress"]["tools"], 2);
    let delegated_item = find(&day, &delegated).unwrap();
    assert_eq!(delegated_item["holder"], child.id.as_str());
    assert_eq!(
        delegated_item["holder_lineage"],
        json!([
            {"id":h.instance,"agent_type":"claude"},
            {"id":child.id,"agent_type":"claude"},
        ]),
        "根→叶"
    );
    assert_eq!(day["items_truncated"], false);
    assert_eq!(day["events_truncated"], false);

    // 一小时窗同样只认窗口内事件；一周窗把三天前那批找回来。
    let hour = h.overview("?window_hours=1").await;
    assert_eq!(hour["items"], day["items"]);
    let week = h.overview("?window_hours=168").await;
    assert_events_consistent(&week);
    let done = find(&week, &stale_done).expect("一周窗内有事件的已结束项");
    assert_eq!(done["status"], "succeeded");
    assert_eq!(done["receipt_status"], "completed");
    let week_events: BTreeSet<&str> = week["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["task_id"].as_str().unwrap())
        .collect();
    for task in [&stale_done, &stale_running, &fresh_done] {
        assert!(week_events.contains(task.as_str().unwrap()), "{task}");
    }
    assert_eq!(week["items"].as_array().unwrap().len(), 5);
    h.stop().await;
}

#[tokio::test]
async fn caps_keep_items_in_project_then_id_order_and_the_newest_events() {
    // 经真实引擎直接在控制库预置 501 个工作项（300 个在 OTHER，201 个在 PROJECT），
    // 再往黑板账本里真实追加 1005 条带任务主体的事件。
    let (h, seeded) = Harness::build(Ledger::Memory, |board, vault| {
        let file = board.join("control.sqlite3");
        let mut engine = PersistentTaskEngine::new(
            ControlDb::open(&file, vault).unwrap(),
            MemoryVaultBroker::with_uuid(vault),
            MemoryAuditLedger::new(),
        );
        let mut seeded: Vec<(String, TaskId)> = Vec::new();
        for index in 0..501 {
            let project = if index < 300 { OTHER } else { PROJECT };
            let spec: BoardTaskSpec = serde_json::from_value(spec(
                project,
                "consumer",
                Some("预置"),
                &format!("seed-{index}"),
            ))
            .unwrap();
            let id = engine
                .create_board_task(&format!("seed-{index}"), spec, &"a".repeat(64), 1)
                .unwrap();
            seeded.push((project.to_owned(), id));
        }
        drop(engine);
        support::mode(&file, 0o600);
        seeded
    })
    .await;
    let target = seeded[0].1;
    let memory = h.memory.as_ref().unwrap();
    for index in 0..1005 {
        memory
            .append(
                AuditEvent::new(
                    AuditEventId::parse(format!("fixture:progress:{index}")).unwrap(),
                    AuditPhase::Outcome,
                    format!("fixture.progress.{index:04}"),
                    SanitizedPayload::try_from(json!({})).unwrap(),
                )
                .with_subject(AuditSubject::default().with_task(target)),
            )
            .unwrap();
    }

    let overview = h.overview("").await;
    let mut expected = seeded.clone();
    expected.sort();
    let expected: Vec<String> = expected[..500]
        .iter()
        .map(|(_, id)| id.to_string())
        .collect();
    let items: Vec<String> = overview["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(items, expected, "按 (项目, id) 取前 500 条");
    assert_eq!(overview["items_truncated"], true);
    assert!(
        overview["items"][0]["project"] == OTHER && overview["items"][499]["project"] == PROJECT
    );

    let actions: Vec<&str> = overview["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["action"].as_str().unwrap())
        .collect();
    let newest: Vec<String> = (5..1005)
        .map(|index| format!("fixture.progress.{index:04}"))
        .collect();
    assert_eq!(actions, newest, "保留最新 1000 条并升序");
    assert_eq!(overview["events_truncated"], true);
    assert!(
        overview["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["task_id"] == target.to_string() && event["project"] == OTHER)
    );
    h.stop().await;
}

/// 畸形或越界的查询串：固定中文 JSON Problem，不回显字段名与解析器细节（T-113 横向写法）。
async fn invalid_query(response: axum::response::Response, query: &str) {
    let body = support::json(response).await;
    assert_eq!(
        body,
        json!({ "code": "invalid_board_overview_query", "error": "查询参数不合法。" }),
        "{query}"
    );
}
