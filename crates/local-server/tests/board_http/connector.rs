//! 连接器（ADR-046）的人侧一半：只读黑板、收会话状态、驾驶舱和 `board presence` 看得到；权限逐项拒绝、
//! 撤了当场没、账本计数、封存冻结。过期与限频的时钟用例在 `board/presence/tests.rs`。
use super::{NOW, PROJECT, TestBoard};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use studiolo_audit_ledger::{AuditRecord, SqliteAuditLedger};
use studiolo_policy::{
    DesiredInput, DesiredStatus, PolicyKind, PolicyStore, RevocationLevel, Submitter,
};

const OTHER: &str = "项目/Other";
const BOARD: &str = "homelab-board";
const STATUS: &str = "homelab-status";
const PRESENCE: &str = "/api/v1/board/presence";

fn publish(policy: &PolicyStore, kind: &str, board: &[&str], presence: &[&str]) {
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
    let desired = policy
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: kind.into(),
                body: Some(set.to_string()),
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

fn retire(policy: &PolicyStore, kind: &str) {
    let desired = policy
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: kind.into(),
                body: None,
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let hash = policy.effective().unwrap().effective_hash;
    policy
        .activate(&desired.id, &hash, "fixture-owner", NOW)
        .unwrap();
}

/// 一块接好两个连接器类型的板。
struct Connected {
    f: TestBoard,
    board: String,
    status: String,
    cookie: String,
}

impl Connected {
    async fn new(board: &[&str], presence: &[&str]) -> Self {
        let f = TestBoard::new().await;
        publish(&f.policy, BOARD, board, &[]);
        publish(&f.policy, STATUS, &[], presence);
        let mut ids = [BOARD, STATUS].map(|kind| {
            f.identities.issue_type(kind, NOW).unwrap();
            f.identities
                .register_instance(kind, "connector", NOW)
                .unwrap()
                .id
        });
        let cookie = f.writer.http.issue().await.cookie;
        Self {
            board: std::mem::take(&mut ids[0]),
            status: std::mem::take(&mut ids[1]),
            cookie,
            f,
        }
    }

    async fn raw(&self, body: String) -> (u16, Value) {
        let reply = self
            .f
            .client
            .post("http://localhost/internal/connector")
            .header("cf-connecting-ip", "127.0.0.1")
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        let status = reply.status().as_u16();
        (
            status,
            serde_json::from_slice(&reply.bytes().await.unwrap()).unwrap(),
        )
    }

    async fn call(
        &self,
        kind: &str,
        instance: &str,
        operation: &str,
        request: Value,
    ) -> (u16, Value) {
        self.call_issued(kind, instance, &self.issued(kind), operation, request)
            .await
    }

    async fn call_issued(
        &self,
        kind: &str,
        instance: &str,
        issued_at: &str,
        operation: &str,
        request: Value,
    ) -> (u16, Value) {
        self.raw(
            json!({"agent_type":kind,"instance_id":instance,"issued_at":issued_at,
                "operation":operation,"request":request})
            .to_string(),
        )
        .await
    }

    /// agent 会报的：这个类型现行令牌的签发时刻。
    fn issued(&self, kind: &str) -> String {
        self.f
            .identities
            .types()
            .unwrap()
            .into_iter()
            .find_map(|(name, issued_at, _)| (name == kind).then_some(issued_at))
            .unwrap()
    }

    async fn list(&self) -> Value {
        let (status, body) = self.call(BOARD, &self.board, "board", json!({})).await;
        assert_eq!(status, 200, "{body}");
        body
    }

    async fn push(&self, seq: u64, sessions: Value) -> (u16, Value) {
        self.call(
            STATUS,
            &self.status,
            "sessions",
            json!({"seq":seq,"sessions":sessions}),
        )
        .await
    }

    async fn seen(&self) -> Vec<Value> {
        self.f.writer.get(PRESENCE, &self.cookie, 200).await["sessions"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn records(&self) -> Vec<AuditRecord> {
        self.f.writer.http.events("board.presence")
    }
}

fn session(index: u64, project: &str) -> Value {
    json!({"kind":"session","key":format!("{index:016x}"),"vendor":"claude_code",
        "project":project,"branch":"claude/canary-branch","state":"running","step":1,"total":4,
        "last_action":"edit","idle_secs":3})
}

fn refused(reply: &(u16, Value), status: u16, code: &str) {
    assert_eq!(reply.0, status, "{}", reply.1);
    assert_eq!(reply.1["code"], code, "{}", reply.1);
}

#[tokio::test]
async fn the_board_list_gives_only_rostered_projects_as_metadata() {
    let c = Connected::new(&[PROJECT, "项目/Empty"], &[PROJECT]).await;
    let before = c.f.writer.http.events("board.request").len();
    let body = c.list().await;
    // 放行没任务是空数组，没放行的项目不出现。
    assert_eq!(
        body["projects"],
        json!([
            {"project":"项目/Empty","items":[],"finished":0,"truncated":false},
            {"project":PROJECT,"items":[],"finished":0,"truncated":false},
        ])
    );
    assert_eq!(
        c.f.writer.http.events("board.request").len(),
        before,
        "连接器读黑板不在人侧账本另记"
    );
    let (task, identity) = c.f.create_claim("connector-1").await;
    c.f.call(
        "return",
        json!({"identity":identity,"receipt":{"status":"completed","artifacts":[],"conflicts":[],
            "unlanded":[],"acceptance":[{"criterion":"证据已落盘","result":"pass","evidence":[]}],
            "remaining":[],"elapsed_ms":100,"tool_calls":1,"learned":[],
            "closeout":{"learned":[],"adr_required":false,"rule_change_required":false,
                "destinations":[]}}}),
        200,
    )
    .await;
    let mut second = c.f.spec();
    second["files_owned"] = json!([format!("{PROJECT}/src")]);
    let open =
        c.f.call(
            "create",
            json!({"request_id":"connector-2","spec":second}),
            200,
        )
        .await;
    let body = c.list().await;
    let project = &body["projects"][1];
    assert_eq!(project["finished"], 1);
    assert_eq!(project["items"].as_array().unwrap().len(), 1);
    let item = &project["items"][0];
    assert_eq!(item["id"], open);
    assert_eq!(item["title"], "写入一份证据");
    let mut names = item
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "adapter",
            "attempt",
            "holder",
            "holder_lineage",
            "id",
            "parent",
            "paths",
            "progress",
            "project",
            "receipt_status",
            "status",
            "title",
        ]
    );
    let text = body.to_string();
    assert!(!text.contains("证据已落盘"), "列表不带验收标准：{text}");
    assert!(!text.contains(task.as_str().unwrap()), "结束的不列：{text}");
    c.f.stop().await;
}

#[tokio::test]
async fn one_task_is_the_same_full_view_as_board_get_and_everything_else_is_not_found() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    let (task, _) = c.f.create_claim("connector-task").await;
    let (status, body) = c
        .call(BOARD, &c.board, "task", json!({"task_id":task}))
        .await;
    assert_eq!(status, 200);
    let full = c.f.call("get", json!({"task_id":task}), 200).await;
    assert_eq!(body, full);
    assert!(body.to_string().contains("证据已落盘"));
    // 不存在与名单外同一句话。
    let missing = c
        .call(
            BOARD,
            &c.board,
            "task",
            json!({"task_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}),
        )
        .await;
    refused(&missing, 404, "connector_not_found");
    publish(&c.f.policy, BOARD, &[OTHER], &[]);
    let outside = c
        .call(BOARD, &c.board, "task", json!({"task_id":task}))
        .await;
    assert_eq!(outside, missing);
    assert_eq!(c.list().await["projects"][0]["project"], OTHER);
    for bad in [
        json!({}),
        json!({"task_id":"x"}),
        json!({"task_id":task,"extra":1}),
    ] {
        refused(
            &c.call(BOARD, &c.board, "task", bad).await,
            422,
            "connector_invalid",
        );
    }
    refused(
        &c.call(BOARD, &c.board, "board", json!({"project":PROJECT}))
            .await,
        422,
        "connector_invalid",
    );
    c.f.stop().await;
}

#[tokio::test]
async fn each_type_reaches_only_its_own_operation_and_nothing_on_the_ordinary_board() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    let snapshot = json!({"seq":1,"sessions":[]});
    for (kind, instance, operation, request) in [
        (STATUS, &c.status, "board", json!({})),
        (
            STATUS,
            &c.status,
            "task",
            json!({"task_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}),
        ),
        (BOARD, &c.board, "sessions", snapshot.clone()),
        ("claude", &c.f.instance, "board", json!({})),
        ("claude", &c.f.instance, "sessions", snapshot.clone()),
        // 拿别的类型的实例号冒充。
        (BOARD, &c.status, "board", json!({})),
        (STATUS, &c.board, "sessions", snapshot.clone()),
    ] {
        refused(
            &c.call(kind, instance, operation, request).await,
            403,
            "connector_denied",
        );
    }
    for operation in ["list", "get", "presence", "create", "claim"] {
        for (kind, instance) in [(BOARD, &c.board), (STATUS, &c.status)] {
            c.f.call_as(kind, instance, PROJECT, operation, json!({}), 409)
                .await;
        }
    }
    // 未知操作、多余字段：固定答复，不回显。
    for body in [
        json!({"agent_type":BOARD,"instance_id":c.board,"issued_at":NOW,"operation":"cards",
            "request":{}}),
        json!({"agent_type":BOARD,"instance_id":c.board,"issued_at":NOW,"operation":"board",
            "request":{},"x":"暗号canary"}),
        // 没带签发时刻的旧 agent：读不懂，不放行。
        json!({"agent_type":BOARD,"instance_id":c.board,"operation":"board","request":{}}),
    ] {
        let reply = c.raw(body.to_string()).await;
        refused(&reply, 422, "connector_invalid");
        assert!(!reply.1.to_string().contains("暗号canary"));
    }
    c.f.stop().await;
}

#[tokio::test]
async fn a_pushed_snapshot_shows_up_in_the_cockpit_and_in_the_board_presence_operation() {
    let c = Connected::new(&[PROJECT], &[PROJECT, OTHER]).await;
    let http = &c.f.writer.http;
    http.send(http.request("GET", PRESENCE, None, ""), 401)
        .await;
    c.f.writer
        .get(&format!("{PRESENCE}?x=1"), &c.cookie, 400)
        .await;
    let empty = c.f.writer.get(PRESENCE, &c.cookie, 200).await;
    assert_eq!(empty["sessions"], json!([]));
    assert_eq!(empty["expires_in_secs"], 300);
    let since = empty["since"].as_str().unwrap().to_owned();
    since.parse::<jiff::Timestamp>().unwrap();

    let (status, body) = c
        .push(
            7,
            json!([
                session(1, PROJECT),
                session(2, OTHER),
                session(3, "项目/Outside")
            ]),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"accepted":2,"rejected":[{"index":2,"reason":"project_not_allowed"}],
            "expires_in_secs":300})
    );
    let rows = c.seen().await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["source"], STATUS);
    assert_eq!(rows[0]["project"], OTHER);
    assert_eq!(rows[1]["branch"], "claude/canary-branch");
    let text = json!(rows).to_string();
    assert!(!text.contains("0000000000000001"), "编号不给：{text}");

    // 别的 AI 只看得到请求的那个项目，且照旧过读白名单、先审计后给。
    let before = http.events("board.request").len();
    let seen = c.f.call("presence", json!({}), 200).await;
    assert_eq!(seen["since"], since);
    assert_eq!(seen["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(seen["sessions"][0]["project"], PROJECT);
    assert_eq!(http.events("board.request").len(), before + 2);
    // 读白名单够了就行，不要黑板写权限。
    let grok =
        c.f.identities
            .register_instance("grok", "reader", NOW)
            .unwrap()
            .id;
    c.f.call_as("grok", &grok, PROJECT, "presence", json!({}), 200)
        .await;
    c.f.call("presence", json!({"x":1}), 409).await;
    c.f.call_as(
        "claude",
        &c.f.instance,
        "项目/Outside",
        "presence",
        json!({}),
        409,
    )
    .await;

    // 旧的 `seq` 不收；字段不合整份不收，上一份不变。
    refused(&c.push(7, json!([])).await, 409, "connector_stale");
    refused(&c.push(6, json!([])).await, 409, "connector_stale");
    let mut bad = session(1, PROJECT);
    bad["command"] = json!("暗号canary");
    let reply = c.push(8, json!([session(2, OTHER), bad])).await;
    refused(&reply, 422, "connector_invalid");
    assert!(!reply.1.to_string().contains("暗号canary"));
    assert_eq!(c.seen().await.len(), 2);
    // 同一会话再推是盖掉，快照里没了的当场消失；空快照就是清空。
    let mut moved = session(1, PROJECT);
    moved["state"] = json!("waiting");
    c.push(8, json!([moved])).await;
    let rows = c.seen().await;
    assert_eq!((rows.len(), &rows[0]["state"]), (1, &json!("waiting")));
    c.push(9, json!([])).await;
    assert!(c.seen().await.is_empty());
    c.f.stop().await;
}

#[tokio::test]
async fn withdrawing_any_grant_removes_the_rows_at_once_and_they_stay_gone() {
    // 收回能力集里一个项目。
    let c = Connected::new(&[PROJECT], &[PROJECT, OTHER]).await;
    let both = json!([session(1, PROJECT), session(2, OTHER)]);
    c.push(1, both.clone()).await;
    publish(&c.f.policy, STATUS, &[], &[PROJECT]);
    assert_eq!(c.seen().await.len(), 1);
    publish(&c.f.policy, STATUS, &[], &[PROJECT, OTHER]);
    assert_eq!(c.seen().await.len(), 1, "再放开也不许带回来");
    // 撤下能力集：整份没有，再推被拒。
    retire(&c.f.policy, STATUS);
    assert!(c.seen().await.is_empty());
    refused(&c.push(2, both.clone()).await, 403, "connector_denied");
    publish(&c.f.policy, STATUS, &[], &[PROJECT, OTHER]);
    assert!(c.seen().await.is_empty());
    // 实例级吊销。
    assert_eq!(c.push(3, both.clone()).await.0, 200);
    assert_eq!(c.seen().await.len(), 2);
    c.f.policy
        .revoke(
            RevocationLevel::Instance,
            &c.status,
            "test",
            "fixture-owner",
            NOW,
        )
        .unwrap();
    assert!(c.seen().await.is_empty());
    c.f.stop().await;

    // 身份库吊销类型（撤令牌）：不等读，巡检一轮也清掉。
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    c.push(1, json!([session(1, PROJECT)])).await;
    assert_eq!(c.seen().await.len(), 1);
    c.f.identities.revoke_type(STATUS, NOW).unwrap();
    assert!(c.seen().await.is_empty());
    refused(
        &c.push(2, json!([session(1, PROJECT)])).await,
        403,
        "connector_denied",
    );
    c.f.identities.revoke_type(BOARD, NOW).unwrap();
    refused(
        &c.call(BOARD, &c.board, "board", json!({})).await,
        403,
        "connector_denied",
    );
    c.f.stop().await;
}

#[tokio::test]
async fn an_unreadable_identity_or_policy_store_gives_nothing_and_answers_unavailable() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    c.push(1, json!([session(1, PROJECT)])).await;
    let issued = [BOARD, STATUS].map(|kind| c.issued(kind));
    type Break = fn(&TestBoard, u32);
    for (store, set_mode) in [
        ("身份库", super::identity_outage::set_mode as Break),
        ("策略库", super::identity_outage::set_policy_mode),
    ] {
        set_mode(&c.f, 0o640);
        c.f.writer.get(PRESENCE, &c.cookie, 503).await;
        let snapshot = json!({"seq":2,"sessions":[]});
        for (kind, instance, issued, operation, request) in [
            (STATUS, &c.status, &issued[1], "sessions", snapshot),
            (BOARD, &c.board, &issued[0], "board", json!({})),
        ] {
            let reply = c
                .call_issued(kind, instance, issued, operation, request)
                .await;
            assert_eq!(reply.0, 503, "{store}：{}", reply.1);
            assert_eq!(reply.1["code"], "board_unavailable", "{store}");
        }
        set_mode(&c.f, 0o600);
        // 读不出来不是撤权：恢复后原来那份还在，没有被当成撤权清掉。
        assert_eq!(c.seen().await.len(), 1, "{store}");
    }
    c.f.stop().await;
}

#[tokio::test]
async fn reissuing_the_token_clears_the_old_rows_and_starts_the_sequence_over() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    assert_eq!(c.push(50, json!([session(1, PROJECT)])).await.0, 200);
    let before = c.issued(STATUS);
    const LATER: &str = "2026-11-25T01:02:03Z";
    c.f.identities.issue_type(STATUS, LATER).unwrap();
    assert_eq!(c.issued(STATUS), LATER);
    assert!(c.seen().await.is_empty(), "换了令牌，旧的那份当场没");
    // 拿旧令牌那一次签发来推：确定拒绝。
    let snapshot = json!({"seq":51,"sessions":[session(1, PROJECT)]});
    refused(
        &c.call_issued(STATUS, &c.status, &before, "sessions", snapshot)
            .await,
        403,
        "connector_denied",
    );
    assert!(c.seen().await.is_empty());
    // 新令牌的计数从头来，推得进去；没有人先读过也一样。
    assert_eq!(c.push(1, json!([session(1, PROJECT)])).await.0, 200);
    assert_eq!(c.seen().await.len(), 1);
    c.f.identities
        .issue_type(STATUS, "2026-11-26T01:02:03Z")
        .unwrap();
    assert_eq!(c.push(1, json!([session(2, PROJECT)])).await.0, 200);
    assert_eq!(c.seen().await.len(), 1);
    c.f.stop().await;
}

#[tokio::test]
async fn the_ledger_gets_one_row_per_new_session_group_and_at_most_one_denial_a_minute() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    for seq in 1..=120u64 {
        let mut rows = (1..=5).map(|i| session(i, PROJECT)).collect::<Vec<_>>();
        rows[0]["step"] = json!(seq % 4);
        assert_eq!(c.push(seq, json!(rows)).await.0, 200);
    }
    let records = c.records();
    assert_eq!(records.len(), 1, "更新不登记");
    let payload = records[0].event().payload().as_value().clone();
    assert_eq!(
        payload,
        json!({"agent_type":STATUS,"agent_instance":c.status,"project":PROJECT,
            "vendor":"claude_code","session_count":5})
    );
    for seq in 121..=140u64 {
        assert_eq!(
            c.push(seq, json!([session(1, "项目/Outside")])).await.0,
            200
        );
        refused(
            &c.call(STATUS, &c.status, "board", json!({})).await,
            403,
            "connector_denied",
        );
    }
    let records = c.records();
    assert_eq!(records.len(), 3, "两种拒绝各一条");
    let text = records
        .iter()
        .map(|record| record.event().payload().as_value().to_string())
        .collect::<String>();
    assert!(
        !text.contains("canary") && !text.contains("Outside"),
        "{text}"
    );
    c.f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn while_the_backup_freezes_the_ledger_only_known_sessions_are_accepted() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    assert_eq!(c.push(1, json!([session(1, PROJECT)])).await.0, 200);
    let ledger = SqliteAuditLedger::open_existing(&c.f.writer.http.audit).unwrap();
    let frozen = ledger
        .quiesce_operations(Instant::now() + Duration::from_secs(2))
        .unwrap();
    let started = Instant::now();
    refused(
        &c.push(2, json!([session(1, PROJECT), session(2, PROJECT)]))
            .await,
        503,
        "board_unavailable",
    );
    let mut known = session(1, PROJECT);
    known["state"] = json!("idle");
    assert_eq!(c.push(3, json!([known])).await.0, 200);
    assert!(started.elapsed() < Duration::from_secs(2), "冻结中不许等");
    drop(frozen);
    let rows = c.seen().await;
    assert_eq!((rows.len(), &rows[0]["state"]), (1, &json!("idle")));
    assert_eq!(
        c.push(4, json!([session(1, PROJECT), session(2, PROJECT)]))
            .await
            .0,
        200
    );
    c.f.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connectors_never_take_more_than_half_of_the_board_permits() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    let issued = c.issued(STATUS);
    // 占住账本的写锁：登记新会话的推送停在半路，占着名额不放。
    let blocker = rusqlite::Connection::open(c.f.writer.http.audit.join("audit.sqlite3")).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut pushes = tokio::task::JoinSet::new();
    for seq in 1..=4u64 {
        let request =
            c.f.client
                .post("http://localhost/internal/connector")
                .header("cf-connecting-ip", "127.0.0.1")
                .header("content-type", "application/json")
                .body(
                    json!({"agent_type":STATUS,"instance_id":c.status,"issued_at":issued,
                        "operation":"sessions",
                        "request":{"seq":seq,"sessions":[session(seq, PROJECT)]}})
                    .to_string(),
                );
        pushes.spawn(async move { request.send().await.unwrap().status().as_u16() });
    }
    for _ in 0..2 {
        let refused = tokio::time::timeout(Duration::from_secs(3), pushes.join_next())
            .await
            .expect("第三、四个没有当场拒")
            .unwrap()
            .unwrap();
        assert_eq!(refused, 429);
    }
    // 剩下的名额还在：网页照常读得到黑板。
    let board = "/api/v1/board?project=%E9%A1%B9%E7%9B%AE%2FStudiolo";
    c.f.writer.get(board, &c.cookie, 200).await;
    blocker.execute_batch("ROLLBACK").unwrap();
    while let Some(status) = pushes.join_next().await {
        assert_ne!(status.unwrap(), 429);
    }
    c.f.stop().await;
}

/// 真的等一轮巡检（15 秒）：没人读的时候撤了权的也清掉，之后再放开不会带回来。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_periodic_check_drops_withdrawn_rows_even_when_nobody_reads() {
    let c = Connected::new(&[PROJECT], &[PROJECT]).await;
    assert_eq!(c.push(1, json!([session(1, PROJECT)])).await.0, 200);
    retire(&c.f.policy, STATUS);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let checks = tokio::spawn(c.f.plane.clone().maintain_until(stopped));
    tokio::time::sleep(Duration::from_secs(18)).await;
    stop.send(true).unwrap();
    checks.await.unwrap().unwrap();
    publish(&c.f.policy, STATUS, &[], &[PROJECT]);
    assert!(c.seen().await.is_empty(), "巡检没有清掉撤了权的");
    c.f.stop().await;
}
