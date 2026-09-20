//! M5真实Unix入口：策略、实例、claim、批量CAS、回执和父吊销；无生产数据。
#[allow(dead_code)]
#[path = "writer_http/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

use serde_json::{Value, json};
use std::{fs, sync::Arc};
use studiolo_local_server::{
    PreparedUnixIngress,
    board::{BoardPlane, private_board_router},
};
use studiolo_policy::{
    DesiredInput, DesiredStatus, PolicyKind, PolicyStore, RevocationLevel, Submitter,
};
use studiolo_supply::AgentStore;
const PROJECT: &str = "项目/Studiolo";
const NOW: &str = "2026-09-20T00:00:00Z";

fn publish(policy: &PolicyStore, kind: &str, writable: bool) {
    let body=json!({"ai_type":kind,"read":{"project_whitelist":[PROJECT,"项目/Other"],"deny_by_default":true},
        "write":{"board":writable,"board_projects":if writable{vec![PROJECT]}else{vec![]},"content_patch_only":true,"content_direct":false},
        "policy":{"propose_desired":true,"activate":false},"hard_bound_skills":[]}).to_string();
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
struct TestBoard {
    writer: fixture::Fixture,
    identities: AgentStore,
    policy: PolicyStore,
    client: reqwest::Client,
    instance: String,
    plane: Arc<BoardPlane>,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
impl TestBoard {
    async fn new() -> Self {
        let mut writer = fixture::Fixture::editable().await;
        let root = &writer.http.root;
        for dir in ["board", "agent", "policy", "board-run"] {
            fs::create_dir(root.join(dir)).unwrap();
            support::mode(&root.join(dir), 0o700);
        }
        for dir in ["03-记忆", "04-协调", "src"] {
            fs::create_dir_all(root.join("vault").join(PROJECT).join(dir)).unwrap();
        }
        fs::write(
            root.join("vault").join(PROJECT).join("04-协调/TASKS.md"),
            "# 人工协调\n\n- 人写值必须保留\n",
        )
        .unwrap();
        let identities = AgentStore::open(&root.join("agent")).unwrap();
        identities.upgrade_child_instances().unwrap();
        identities.issue_type("claude", NOW).unwrap();
        identities.issue_type("grok", NOW).unwrap();
        let instance = identities
            .register_instance("claude", "test-parent", NOW)
            .unwrap()
            .id;
        let policy = PolicyStore::open(&root.join("policy")).unwrap();
        publish(&policy, "claude", true);
        publish(&policy, "grok", false);
        let audit = Arc::new(
            studiolo_audit_ledger::SqliteAuditLedger::open_existing(&writer.http.audit).unwrap(),
        );
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
        writer.http.app =
            studiolo_local_server::board::with_board_plane(writer.http.app, plane.clone());
        Self {
            writer,
            identities,
            policy,
            client,
            instance,
            plane,
            stop,
            server,
        }
    }
    async fn call_as(
        &self,
        kind: &str,
        instance: &str,
        project: &str,
        op: &str,
        request: Value,
        status: u16,
    ) -> Value {
        let reply=self.client.post("http://localhost/internal/board").header("cf-connecting-ip","127.0.0.1")
            .header("content-type","application/json").body(json!({"agent_type":kind,"instance_id":instance,"project":project,"operation":op,"request":request}).to_string()).send().await.unwrap();
        let got = reply.status().as_u16();
        let body: Value = serde_json::from_slice(&reply.bytes().await.unwrap()).unwrap();
        assert_eq!(got, status, "{body}");
        body
    }
    async fn call(&self, op: &str, request: Value, status: u16) -> Value {
        self.call_as("claude", &self.instance, PROJECT, op, request, status)
            .await
    }
    fn spec(&self) -> Value {
        json!({"task":{"role":"author","adapter":"consumer","input_paths":[],"allow_web":[],"allow_db":[],
        "retry_policy":{"max_attempts":1,"backoff_seconds":0,"retry_on":[]},"timeout":300,"project":PROJECT,"parent":null,"title":"写入一份证据","instruction":null},
        "acceptance_criteria":["证据已落盘"],"files_owned":[format!("{PROJECT}/src"),format!("{PROJECT}/03-记忆")],"max_children":8,"artifact_target":null})
    }
    async fn create_claim(&self, key: &str) -> (Value, Value) {
        let task = self
            .call("create", json!({"request_id":key,"spec":self.spec()}), 200)
            .await;
        let until = jiff::Timestamp::now().as_millisecond() + 60_000;
        let claim = self
            .call("claim", json!({"task_id":task,"lease_until_ms":until}), 200)
            .await;
        let identity = json!({"task_id":task,"attempt_id":claim["attempt"]["attempt_id"],"fence":claim["attempt"]["fence"]});
        (task, identity)
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

#[tokio::test]
async fn human_execution_is_session_csrf_and_l3_guarded_and_default_frozen() {
    let f = TestBoard::new().await;
    let http = &f.writer.http;
    http.send(http.request("GET", "/api/v1/execution", None, ""), 401)
        .await;
    let login = http.issue().await;
    let body = f.writer.get("/api/v1/execution", &login.cookie, 200).await;
    assert_eq!(body, json!({"configured":false,"frozen":true,"runs":[]}));
    let (task, _) = f.create_claim("human-audit").await;
    let board = f
        .writer
        .get(
            "/api/v1/board?project=%E9%A1%B9%E7%9B%AE%2FStudiolo",
            &login.cookie,
            200,
        )
        .await;
    assert_eq!(board.as_array().unwrap().len(), 1);
    assert_eq!(board[0]["instance_lineages"][0][0]["agent_type"], "claude");
    let replay = f
        .writer
        .get(
            &format!("/api/v1/board/replay?task_id={}", task.as_str().unwrap()),
            &login.cookie,
            200,
        )
        .await;
    assert_eq!(replay["task"]["task"]["task_id"], task);
    assert!(replay.get("events").unwrap().is_array());
    // Project replay is a session-protected join of audit metadata and fresh log pointers.
    f.writer
        .get("/api/workbench?refresh=true", &login.cookie, 200)
        .await;
    let now = jiff::Timestamp::now();
    let from = jiff::Timestamp::from_second(now.as_second() - 86400).unwrap();
    let to = jiff::Timestamp::from_second(now.as_second() + 60).unwrap();
    let timeline =
        format!("/api/v1/board/timeline?project=%E9%A1%B9%E7%9B%AE%2FStudiolo&from={from}&to={to}");
    http.send(http.request("GET", &timeline, None, ""), 401)
        .await;
    let page = f.writer.get(&timeline, &login.cookie, 200).await;
    assert_eq!(page["project"], PROJECT);
    assert!(!page["events"].as_array().unwrap().is_empty());
    assert!(
        page["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["task_id"] == task && e.get("payload").is_none())
    );
    let foreign = f
        .writer
        .get(&timeline.replace("Studiolo", "Other"), &login.cookie, 200)
        .await;
    assert!(foreign["events"].as_array().unwrap().is_empty());
    http.send(
        http.request(
            "GET",
            &timeline.replace(&format!("from={from}"), "from=invalid"),
            Some(&login.cookie),
            "",
        ),
        400,
    )
    .await;
    http.send(
        http.request(
            "GET",
            &timeline.replace(&format!("from={from}"), "from=2020-01-01T00:00:00Z"),
            Some(&login.cookie),
            "",
        ),
        400,
    )
    .await;
    let command = "/api/v1/execution/command";
    http.send(http.request("GET", command, Some(&login.cookie), ""), 405)
        .await;
    http.send(
        http.request(
            "POST",
            command,
            Some(&login.cookie),
            r#"{"operation":"freeze","frozen":false}"#,
        ),
        403,
    )
    .await;
    let mut request = http.request(
        "POST",
        command,
        Some(&login.cookie),
        r#"{"operation":"freeze","frozen":false}"#,
    );
    request
        .headers_mut()
        .insert("x-csrf-token", login.csrf.parse().unwrap());
    let denial = support::json(http.send(request, 409).await).await;
    assert!(denial["error"].as_str().unwrap().contains("二次认证"));
    assert_eq!(http.events("execution.unfreeze").len(), 2);
    f.stop().await;
}

#[tokio::test]
async fn claim_batch_preview_commit_receipt_and_projection_are_one_authenticated_flow() {
    let f = TestBoard::new().await;
    let mut missing = f.spec();
    missing
        .as_object_mut()
        .unwrap()
        .remove("acceptance_criteria");
    f.call("create", json!({"request_id":"bad","spec":missing}), 409)
        .await;
    let (task, identity) = f.create_claim("create-1").await;
    let path = format!("{PROJECT}/src/evidence.md");
    let batch = json!({"batch_id":"proof-1","changes":[{"path":path,"edit":{"kind":"create","content":"# 已验证证据\n"}}]});
    let plan = f
        .call(
            "batch.preview",
            json!({"identity":identity,"batch":batch}),
            200,
        )
        .await;
    let result = f
        .call(
            "batch.commit",
            json!({"identity":identity,"batch":batch,"preview":plan["preview"]}),
            200,
        )
        .await;
    assert!(!result["receipt"]["rolled_back"].as_bool().unwrap());
    assert_eq!(
        fs::read_to_string(f.writer.http.root.join("vault").join(&path)).unwrap(),
        "# 已验证证据\n"
    );
    // 真实Broker读取指针来自文件；测试不自行捏造成功哈希。
    use studiolo_vault_broker::CanonicalReadVault;
    let object = CanonicalReadVault::open_existing(&f.writer.http.root.join("vault"))
        .unwrap()
        .read_file(&studiolo_core::VaultPath::parse(&path).unwrap(), 4096)
        .unwrap();
    use sha2::{Digest, Sha256};
    let sha = Sha256::digest(&object.content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let pointer = json!({"path":path,"revision":object.revision.to_string(),"sha256":sha});
    let mut receipt = json!({"status":"completed","artifacts":[pointer.clone()],"conflicts":[],"unlanded":[],
        "acceptance":[{"criterion":"证据已落盘","result":"pass","evidence":[pointer]}],"remaining":[],"elapsed_ms":100,"tool_calls":2,"learned":[]});
    f.call(
        "return",
        json!({"identity":identity,"receipt":receipt}),
        409,
    )
    .await;
    receipt["closeout"] =
        json!({"learned":[],"adr_required":false,"rule_change_required":false,"destinations":[]});
    f.call(
        "return",
        json!({"identity":identity,"receipt":receipt}),
        200,
    )
    .await;
    let view = f.call("get", json!({"task_id":task}), 200).await;
    assert_eq!(view["task"]["status"], "succeeded");
    let projection = fs::read_to_string(
        f.writer
            .http
            .root
            .join("vault")
            .join(PROJECT)
            .join("04-协调/TASKS.md"),
    )
    .unwrap();
    assert!(projection.contains("人写值必须保留") && projection.contains("succeeded"));
    f.call(
        "batch.commit",
        json!({"identity":identity,"batch":batch,"preview":plan["preview"]}),
        409,
    )
    .await;
    f.stop().await;
}

#[tokio::test]
async fn read_only_cross_project_parent_revocation_and_memory_roundtrip() {
    let f = TestBoard::new().await;
    let grok = f
        .identities
        .register_instance("grok", "read-only", NOW)
        .unwrap();
    f.call_as("grok", &grok.id, PROJECT, "list", json!({}), 200)
        .await;
    f.call_as(
        "grok",
        &grok.id,
        PROJECT,
        "create",
        json!({"request_id":"denied","spec":f.spec()}),
        409,
    )
    .await;
    f.call_as("claude", &f.instance, "项目/Other", "list", json!({}), 200)
        .await;
    let mut readonly_spec = f.spec();
    readonly_spec["task"]["project"] = "项目/Other".into();
    readonly_spec["files_owned"] = json!([]);
    f.call_as(
        "claude",
        &f.instance,
        "项目/Other",
        "create",
        json!({"request_id":"readonly-project","spec":readonly_spec}),
        409,
    )
    .await;
    let mut escaped = f.spec();
    escaped["files_owned"] = json!(["项目/Other"]);
    f.call(
        "create",
        json!({"request_id":"escaped","spec":escaped}),
        409,
    )
    .await;
    let (_, identity) = f.create_claim("memory-task").await;
    let memory=f.call("memory.preview",json!({"identity":identity,"entry":{"batch_id":"memory-1","scope":format!("{PROJECT}/03-记忆"),
        "kind":"reference","summary":"黑板示例","date":"2026-09-20","body":"# 黑板示例\n\n只用于合成测试。\n"}}),200).await;
    assert_eq!(memory["kind"], "create");
    assert_eq!(memory["request"]["batch_id"], "memory-1");
    f.call(
        "batch.commit",
        json!({"identity":identity,"batch":memory["request"],"preview":memory["preview"]}),
        200,
    )
    .await;
    let child = f
        .identities
        .register_child_instance("claude", &f.instance, "child", NOW)
        .unwrap();
    f.call_as("claude", &child.id, PROJECT, "list", json!({}), 200)
        .await;
    f.policy
        .revoke(
            RevocationLevel::Instance,
            &f.instance,
            "test revocation",
            "fixture-owner",
            NOW,
        )
        .unwrap();
    f.call_as("claude", &child.id, PROJECT, "list", json!({}), 409)
        .await;
    f.stop().await;
}

#[tokio::test]
async fn historical_holder_cannot_cancel_a_successor_after_handoff() {
    let f = TestBoard::new().await;
    let (task, old) = f.create_claim("handoff-cancel").await;
    let view = f.call("get", json!({"task_id":task}), 200).await;
    let handoff = json!({"goal":"继续验证","constraints":[{"rule_id":"fixture","version_hash":view["board"]["definition"]["policy_hash"]}],"decided":[],"undecided":[],"required_files":[],"acceptance_criteria":["证据已落盘"],"forbidden":["不得覆盖他人正文"]});
    f.call("handoff", json!({"identity":old,"handoff":handoff}), 200)
        .await;
    let successor = f
        .identities
        .register_instance("claude", "successor", NOW)
        .unwrap();
    let claim = f
        .call_as(
            "claude",
            &successor.id,
            PROJECT,
            "claim",
            json!({"task_id":task,"lease_until_ms":jiff::Timestamp::now().as_millisecond()+60_000}),
            200,
        )
        .await;
    f.call("cancel", old, 409).await;
    let after = f.call("get", json!({"task_id":task}), 200).await;
    assert_eq!(after["task"]["status"], "running");
    assert_eq!(
        after["task"]["active_attempt"],
        claim["attempt"]["attempt_id"]
    );
    f.stop().await;
}
