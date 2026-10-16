//! T-182 接入方提案的断线重提（真实 loopback JSON-RPC）：同类型换实例重提同一份正文不新建、返回同一
//! desired_id 并带服务端算的规范化正文摘要；本类型在该 subject 之后提过别的草案时重提照常新建，别的类型的
//! 草案不算；提交审计落不了账时如实说已入库，恢复后重提拿回它并补记；`list_proposals` 只列本类型仍待处理的
//! 提案、先记读取审计、需提案能力、每次重验吊销；未接入策略平面时与 `policy_read` 同样拒绝。

use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_policy::{
    DesiredInput, DesiredStatus, MAX_LISTED_PROPOSALS, MAX_PENDING_PER_INSTANCE, PolicyKind,
    PolicyStore, RevocationLevel, Submitter, sha256_hex,
};
use studiolo_supply::{AgentStore, ReadQuery};
use tempfile::TempDir;

const NOW: &str = "2026-10-15T20:04:20Z";
const TYPES: [&str; 2] = ["claude", "codex"];

struct Fixture {
    _temp: TempDir,
    data: std::path::PathBuf,
    policy: std::path::PathBuf,
    tokens: HashMap<&'static str, String>,
    base: String,
    client: reqwest::Client,
}

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn capability(ai_type: &str, propose: bool) -> String {
    json!({
        "ai_type": ai_type,
        "read": {"project_whitelist": ["项目/Alpha"], "deny_by_default": true, "rules_whitelist": []},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": propose, "activate": false},
        "hard_bound_skills": [],
    })
    .to_string()
}

fn publish_capability(store: &PolicyStore, ai_type: &str, propose: bool) {
    let record = store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: ai_type.into(),
                body: Some(capability(ai_type, propose)),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    assert_eq!(
        record.status,
        DesiredStatus::Validated,
        "{:?}",
        record.report
    );
    let hash = store.effective().unwrap().effective_hash;
    store.activate(&record.id, &hash, "owner", NOW).unwrap();
}

/// 起一个真实 agent 服务；`with_policy` 为假时是未接入策略平面的 M3 兼容模式。
async fn fixture(with_policy: bool) -> Fixture {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 🟡 进行中 |\n| 最近日志 | 无 |\n| 未关闭 BUG | 无 |\n",
    );
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));
    let agents = AgentStore::open(&data).unwrap();
    let tokens = TYPES
        .iter()
        .map(|name| (*name, agents.issue_type(name, NOW).unwrap().token))
        .collect::<HashMap<_, _>>();
    drop(agents);
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());
    let policy = temp.path().join("policy-data");
    if with_policy {
        private_dir(&policy);
        let store = PolicyStore::open(&policy).unwrap();
        for name in TYPES {
            publish_capability(&store, name, true);
        }
    }
    let state = serve::prepare(&ServeOptions {
        board_socket: None,
        index_cache: None,
        vault: Some(vault),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into()],
        data: data.clone(),
        policy: with_policy.then(|| policy.clone()),
        port: 0,
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = serve::router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        _temp: temp,
        data,
        policy,
        tokens,
        base: format!("http://127.0.0.1:{port}/mcp"),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
    }
}

impl Fixture {
    fn policy(&self) -> PolicyStore {
        PolicyStore::open_existing(&self.policy).unwrap()
    }

    fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }

    /// 某个审计动作的全部 payload，按落账先后。
    fn events(&self, action: &str) -> Vec<Value> {
        self.ledger()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
            .iter()
            .map(|record| record.event().payload().as_value().clone())
            .collect()
    }

    fn denied_reasons(&self) -> Vec<String> {
        self.events("agent.action_denied")
            .iter()
            .map(|payload| payload["reason"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn rpc(
        &self,
        agent: &str,
        session: Option<&str>,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Option<String>, Value) {
        let mut request = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Authorization", format!("Bearer {}", self.tokens[agent]))
            .body(
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
            );
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        let mut response = request.send().await.unwrap();
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
                    .collect::<String>();
                if let Ok(value) = serde_json::from_str::<Value>(&data)
                    && value.get("id") == Some(&json!(id))
                {
                    return (session_id, value);
                }
            }
        }
        panic!("no JSON-RPC response for {method}: {buffer}");
    }

    /// 握手并完成 initialized；返回 (会话, 实例)。每次握手都是一个新实例，即断线重连后的样子。
    async fn initialize(&self, agent: &str) -> (String, String) {
        let params = json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "proposal-contract", "version": "0"}});
        let (session, response) = self.rpc(agent, None, 1, "initialize", params).await;
        let session = session.expect("session id header");
        let status = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Authorization", format!("Bearer {}", self.tokens[agent]))
            .header("Mcp-Session-Id", &session)
            .body(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 202);
        let instructions = response["result"]["instructions"].as_str().unwrap();
        let instance = instructions
            .split("实例 ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_owned();
        (session, instance)
    }

    async fn call(
        &self,
        agent: &str,
        session: &str,
        id: u64,
        name: &str,
        arguments: Value,
    ) -> Value {
        self.rpc(
            agent,
            Some(session),
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
        .1
    }

    /// 提交一份规则母本提案，返回成功时的文本。
    async fn propose_rule(
        &self,
        agent: &str,
        session: &str,
        id: u64,
        subject: &str,
        body: &str,
    ) -> String {
        let response = self
            .call(
                agent,
                session,
                id,
                "propose_desired",
                json!({"kind": "rules", "subject": subject, "body": body}),
            )
            .await;
        assert!(!is_error(&response), "{response}");
        tool_text(&response)
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

/// `propose_desired` 返回里「键 值」一行的值。
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
}

/// `list_proposals` 的数据行：desired_id、kind、subject、状态、摘要、提交时间、实例。
fn listed(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|line| line.contains('\t'))
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

#[tokio::test]
async fn a_retry_from_a_new_instance_returns_the_same_pending_proposal() {
    let f = fixture(true).await;
    let (first, first_instance) = f.initialize("claude").await;
    let crlf = "# 提议\r\n\r\n断线前提交的正文。\r\n";
    let lf = crlf.replace("\r\n", "\n");
    let digest = sha256_hex(lf.as_bytes());
    let text = f.propose_rule("claude", &first, 2, "提议.md", crlf).await;
    let desired_id = field(&text, "desired_id").unwrap().to_owned();
    assert_eq!(field(&text, "status"), Some("pending"), "{text}");
    assert_eq!(field(&text, "body_sha256"), Some(digest.as_str()), "{text}");
    assert_eq!(field(&text, "deduplicated"), Some("false"), "{text}");
    assert!(text.contains("永不自动生效"), "{text}");

    // 断线重连后是新会话、新实例；正文换成 LF 也是同一份规范化正文。
    let (second, second_instance) = f.initialize("claude").await;
    assert_ne!(first_instance, second_instance);
    let text = f.propose_rule("claude", &second, 2, "提议.md", &lf).await;
    assert_eq!(
        field(&text, "desired_id"),
        Some(desired_id.as_str()),
        "{text}"
    );
    assert_eq!(field(&text, "status"), Some("pending"), "{text}");
    assert_eq!(field(&text, "ok"), Some("true"), "{text}");
    assert_eq!(field(&text, "body_sha256"), Some(digest.as_str()), "{text}");
    assert_eq!(field(&text, "deduplicated"), Some("true"), "{text}");

    // 库里只有一条，仍记在第一次提交的实例名下，入库的是第一次的原文。
    let rows = f
        .policy()
        .desired_list(None, 100)
        .unwrap()
        .into_iter()
        .filter(|record| record.subject == "提议.md")
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].id, desired_id);
    assert_eq!(rows[0].status, DesiredStatus::Pending);
    assert_eq!(
        rows[0].agent_instance.as_deref(),
        Some(first_instance.as_str())
    );
    assert_eq!(rows[0].body.as_deref(), Some(crlf));
    assert_eq!(rows[0].body_sha256, digest);

    // 每次调用各记一条提交审计；第二条标明命中已有提案，主体是重提的实例。
    let submitted = f.events("policy.desired_submitted");
    let agent_events = submitted
        .iter()
        .filter(|payload| payload["actor"] == "agent")
        .collect::<Vec<_>>();
    assert_eq!(agent_events.len(), 2, "{submitted:?}");
    assert_eq!(agent_events[0]["deduplicated"], false);
    assert_eq!(agent_events[0]["agent_instance"], first_instance);
    assert_eq!(agent_events[1]["deduplicated"], true);
    assert_eq!(agent_events[1]["agent_instance"], second_instance);
    assert_eq!(agent_events[1]["desired_id"], desired_id);
    assert!(
        agent_events
            .iter()
            .all(|payload| payload.get("body").is_none())
    );
}

#[tokio::test]
async fn only_a_newer_draft_of_the_same_type_turns_a_resubmission_into_a_new_proposal() {
    let f = fixture(true).await;
    let (session, _) = f.initialize("claude").await;
    let first = f
        .propose_rule("claude", &session, 2, "顺序.md", "# 第一版\n")
        .await;
    let second = f
        .propose_rule("claude", &session, 3, "顺序.md", "# 第二版\n")
        .await;
    let first_id = field(&first, "desired_id").unwrap().to_owned();
    let second_id = field(&second, "desired_id").unwrap().to_owned();
    assert_ne!(first_id, second_id);
    assert_eq!(field(&second, "deduplicated"), Some("false"));

    // 本类型在第一版之后又提了第二版：再提第一版是要它重新成为本类型最新的提议，所以新建，不拿回旧的。
    let again = f
        .propose_rule("claude", &session, 4, "顺序.md", "# 第一版\n")
        .await;
    let again_id = field(&again, "desired_id").unwrap().to_owned();
    assert!(again_id != first_id && again_id != second_id, "{again}");
    assert_eq!(field(&again, "deduplicated"), Some("false"), "{again}");
    // 它现在是最新的，再重提就拿回它。
    let retried = f
        .propose_rule("claude", &session, 5, "顺序.md", "# 第一版\n")
        .await;
    assert_eq!(
        field(&retried, "desired_id"),
        Some(again_id.as_str()),
        "{retried}"
    );
    assert_eq!(field(&retried, "deduplicated"), Some("true"), "{retried}");

    // 别的类型提交同一正文是另一个主体的提案，照常新建。
    let (codex, _) = f.initialize("codex").await;
    let other = f
        .propose_rule("codex", &codex, 2, "顺序.md", "# 第一版\n")
        .await;
    assert_ne!(
        field(&other, "desired_id"),
        Some(again_id.as_str()),
        "{other}"
    );
    assert_eq!(field(&other, "deduplicated"), Some("false"), "{other}");
    // codex 那份比它新，但重提只看本类型自己的草案：claude 再提照样拿回自己那份，不多出一条。
    let (reconnected, _) = f.initialize("claude").await;
    let mine = f
        .propose_rule("claude", &reconnected, 2, "顺序.md", "# 第一版\n")
        .await;
    assert_eq!(
        field(&mine, "desired_id"),
        Some(again_id.as_str()),
        "{mine}"
    );
    assert_eq!(field(&mine, "deduplicated"), Some("true"), "{mine}");
    let pending = f
        .policy()
        .desired_list(Some(DesiredStatus::Pending), 100)
        .unwrap()
        .into_iter()
        .filter(|record| record.subject == "顺序.md")
        .count();
    assert_eq!(pending, 4);
}

#[tokio::test]
async fn a_body_that_cannot_be_canonicalized_has_no_digest_and_is_never_deduplicated() {
    let f = fixture(true).await;
    let (session, _) = f.initialize("claude").await;
    let mut ids = Vec::new();
    for id in 2..4 {
        let response = f
            .call(
                "claude",
                &session,
                id,
                "propose_desired",
                json!({"kind": "capability_set", "subject": "claude", "body": "{}"}),
            )
            .await;
        let text = tool_text(&response);
        assert_eq!(field(&text, "status"), Some("rejected"), "{response}");
        assert_eq!(field(&text, "body_sha256"), Some("-"), "{text}");
        assert_eq!(field(&text, "deduplicated"), Some("false"), "{text}");
        ids.push(field(&text, "desired_id").unwrap().to_owned());
    }
    assert_ne!(ids[0], ids[1]);
}

#[tokio::test]
async fn a_canonical_body_rejected_on_its_merits_shows_its_digest_but_is_never_deduplicated() {
    let f = fixture(true).await;
    let (session, _) = f.initialize("claude").await;
    // 结构合法、能规范化，但白名单之外放行：语义校验拒绝，摘要仍是规范化正文的真摘要。
    let mut set: Value = serde_json::from_str(&capability("claude", true)).unwrap();
    set["read"]["deny_by_default"] = json!(false);
    let body = set.to_string();
    let mut submitted = Vec::new();
    for id in 2..4 {
        let response = f
            .call(
                "claude",
                &session,
                id,
                "propose_desired",
                json!({"kind": "capability_set", "subject": "claude", "body": body}),
            )
            .await;
        let text = tool_text(&response);
        assert_eq!(field(&text, "status"), Some("rejected"), "{response}");
        assert_eq!(field(&text, "deduplicated"), Some("false"), "{text}");
        let digest = field(&text, "body_sha256").unwrap().to_owned();
        assert!(
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{text}"
        );
        assert!(text.contains("read.deny_by_default"), "{text}");
        submitted.push((field(&text, "desired_id").unwrap().to_owned(), digest));
    }
    // 被拒的不拿回，重提新建一条；两次摘要相同，且就是入库的那个。
    assert_ne!(submitted[0].0, submitted[1].0);
    assert_eq!(submitted[0].1, submitted[1].1);
    let stored = f.policy().desired(&submitted[0].0).unwrap().unwrap();
    assert_eq!(stored.body_sha256, submitted[0].1);
}

#[tokio::test]
async fn an_audit_outage_after_the_insert_says_it_is_stored_and_a_retry_records_it() {
    let f = fixture(true).await;
    let (first, first_instance) = f.initialize("claude").await;
    // 账本目录权限被破坏：提案照常入库，提交审计落不了账。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    let failed = f
        .call(
            "claude",
            &first,
            2,
            "propose_desired",
            json!({"kind": "rules", "subject": "断审计.md", "body": "# 断审计\n"}),
        )
        .await;
    let rows = f
        .policy()
        .desired_list(None, 100)
        .unwrap()
        .into_iter()
        .filter(|record| record.subject == "断审计.md")
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let desired_id = rows[0].id.clone();
    assert_eq!(rows[0].status, DesiredStatus::Pending);
    assert_eq!(
        rows[0].agent_instance.as_deref(),
        Some(first_instance.as_str())
    );
    // 如实说已入库并给出 desired_id，不能让提交方以为没进库。
    let message = failed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("审计不可用")
            && message.contains("已入库")
            && message.contains(&desired_id),
        "{failed}"
    );
    // 账本仍坏着时，列表的读取审计也落不了账：失败关闭，一行都不给。
    let unlisted = f
        .call("claude", &first, 3, "list_proposals", json!({}))
        .await;
    assert!(unlisted.get("result").is_none(), "{unlisted}");
    assert!(
        unlisted["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("审计不可用"),
        "{unlisted}"
    );
    assert!(!unlisted.to_string().contains(&desired_id), "{unlisted}");

    // 恢复后重连、重提同一份：拿回它，这一条提交审计补记在重提的实例名下。
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    let (second, second_instance) = f.initialize("claude").await;
    let text = f
        .propose_rule("claude", &second, 2, "断审计.md", "# 断审计\n")
        .await;
    assert_eq!(
        field(&text, "desired_id"),
        Some(desired_id.as_str()),
        "{text}"
    );
    assert_eq!(field(&text, "deduplicated"), Some("true"), "{text}");
    let recorded = f
        .events("policy.desired_submitted")
        .into_iter()
        .filter(|payload| payload["actor"] == "agent")
        .collect::<Vec<_>>();
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0]["desired_id"], desired_id);
    assert_eq!(recorded[0]["deduplicated"], true);
    assert_eq!(recorded[0]["agent_instance"], second_instance);
    // 也能用 list_proposals 核对。
    let listing = f
        .call("claude", &second, 3, "list_proposals", json!({}))
        .await;
    let ids = listed(&tool_text(&listing))
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, [desired_id], "{listing}");
}

#[tokio::test]
async fn list_proposals_shows_only_the_callers_open_proposals_and_is_read_audited() {
    let f = fixture(true).await;
    let store = f.policy();
    let (session, instance) = f.initialize("claude").await;
    let id_of = |text: String| field(&text, "desired_id").unwrap().to_owned();
    let a = id_of(f.propose_rule("claude", &session, 2, "a.md", "# A\n").await);
    let b = id_of(f.propose_rule("claude", &session, 3, "b.md", "# B\n").await);
    let c = id_of(f.propose_rule("claude", &session, 4, "c.md", "# C\n").await);
    let d = id_of(f.propose_rule("claude", &session, 5, "d.md", "# D\n").await);
    let rejected = f
        .call(
            "claude",
            &session,
            6,
            "propose_desired",
            json!({"kind": "capability_set", "subject": "claude", "body": "{}"}),
        )
        .await;
    assert_eq!(field(&tool_text(&rejected), "status"), Some("rejected"));
    // 主人撤回 c、复核 d；主人自己提交的 e 不是接入方提案。
    store.withdraw_desired(&c, NOW).unwrap();
    assert_eq!(
        store.validate_desired(&d, NOW).unwrap().status,
        DesiredStatus::Validated
    );
    store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "e.md".into(),
                body: Some("# E\n".into()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    let (codex, _) = f.initialize("codex").await;
    let x = id_of(f.propose_rule("codex", &codex, 2, "a.md", "# A\n").await);

    // 断线重连后的新实例照样看得到本类型的提案：按类型列，不按实例。
    let (reader, reader_instance) = f.initialize("claude").await;
    let response = f
        .call("claude", &reader, 2, "list_proposals", json!({}))
        .await;
    assert!(!is_error(&response), "{response}");
    let text = tool_text(&response);
    assert!(text.starts_with("# 待处理提案"), "{text}");
    let rows = listed(&text);
    assert_eq!(
        rows.iter().map(|row| row[0].clone()).collect::<Vec<_>>(),
        [d.as_str(), b.as_str(), a.as_str()],
        "{text}"
    );
    let row_a = rows.iter().find(|row| row[0] == a).unwrap();
    assert_eq!(row_a.len(), 7, "{text}");
    assert_eq!(row_a[1], "rules");
    assert_eq!(row_a[2], "a.md");
    assert_eq!(row_a[3], "pending");
    assert_eq!(row_a[4], sha256_hex(b"# A\n"));
    assert!(!row_a[5].is_empty());
    assert_eq!(row_a[6], instance);
    assert_eq!(rows.iter().find(|row| row[0] == d).unwrap()[3], "validated");

    // 按 kind／subject 收窄。
    let narrowed = f
        .call(
            "claude",
            &reader,
            3,
            "list_proposals",
            json!({"kind": "rules", "subject": "a.md"}),
        )
        .await;
    let narrowed_rows = listed(&tool_text(&narrowed));
    assert_eq!(narrowed_rows.len(), 1, "{narrowed}");
    assert_eq!(narrowed_rows[0][0], a);
    let skills = f
        .call(
            "claude",
            &reader,
            4,
            "list_proposals",
            json!({"kind": "skills"}),
        )
        .await;
    assert!(!is_error(&skills), "{skills}");
    assert!(listed(&tool_text(&skills)).is_empty(), "{skills}");
    let bad_kind = f
        .call(
            "claude",
            &reader,
            5,
            "list_proposals",
            json!({"kind": "nope"}),
        )
        .await;
    assert_eq!(bad_kind["error"]["code"], -32602, "{bad_kind}");

    // 别的类型只看到自己的。
    let other = f
        .call("codex", &codex, 3, "list_proposals", json!({}))
        .await;
    let other_rows = listed(&tool_text(&other));
    assert_eq!(
        other_rows
            .iter()
            .map(|row| row[0].clone())
            .collect::<Vec<_>>(),
        [x.as_str()],
        "{other}"
    );

    // 每次成功列出都先记读取审计并进四维读取索引。
    let reads = f
        .events("content.read")
        .into_iter()
        .filter(|payload| payload["tool"] == "list_proposals")
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 4, "{reads:?}");
    assert!(
        reads
            .iter()
            .all(|payload| payload["path"] == "policy://desired")
    );
    let indexed = AgentStore::open_existing(&f.data)
        .unwrap()
        .query_reads(&ReadQuery {
            instance_id: Some(reader_instance),
            path_prefix: Some("policy://desired".into()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(indexed.len(), 3);
    assert!(indexed.iter().all(|record| record.tool == "list_proposals"));
}

#[tokio::test]
async fn list_proposals_stops_at_the_listing_bound_and_says_there_are_more() {
    let f = fixture(true).await;
    let store = f.policy();
    // 比列表上限多一份；每实例的待处理名额有限，分给几个实例。
    for index in 0..=MAX_LISTED_PROPOSALS {
        store
            .submit_desired(
                DesiredInput {
                    kind: PolicyKind::Rules,
                    subject: format!("r{index:02}.md"),
                    body: Some(format!("# r{index:02}\n")),
                },
                Submitter::Agent {
                    agent_type: "claude".into(),
                    instance_id: format!(
                        "01FIXTUREINSTANCE{:02}",
                        index / MAX_PENDING_PER_INSTANCE
                    ),
                },
                NOW,
            )
            .unwrap();
    }
    let (session, _) = f.initialize("claude").await;
    let response = f
        .call("claude", &session, 2, "list_proposals", json!({}))
        .await;
    let text = tool_text(&response);
    let rows = listed(&text);
    assert_eq!(rows.len(), MAX_LISTED_PROPOSALS, "{text}");
    assert_eq!(rows[0][2], format!("r{MAX_LISTED_PROPOSALS:02}.md"));
    assert!(rows.iter().all(|row| row[2] != "r00.md"));
    assert!(
        text.lines().next().unwrap().contains("还有更早的"),
        "{text}"
    );

    // 工具说明里的份数是属性里的字面量，这里把它钉在常量上：改常量忘了改说明就红。
    let (_, tools) = f
        .rpc("claude", Some(&session), 3, "tools/list", json!({}))
        .await;
    let description = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "list_proposals")
        .unwrap()["description"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        description.contains(&format!("最多 {MAX_LISTED_PROPOSALS} 份")),
        "{description}"
    );
}

#[tokio::test]
async fn list_proposals_needs_the_propose_capability() {
    let f = fixture(true).await;
    publish_capability(&f.policy(), "claude", false);
    let (session, _) = f.initialize("claude").await;
    let refused = f
        .call("claude", &session, 2, "list_proposals", json!({}))
        .await;
    assert!(refused.get("result").is_none(), "{refused}");
    assert_eq!(refused["error"]["code"], -32600, "{refused}");
    assert_eq!(f.denied_reasons().last().unwrap(), "capability_denied");
    assert!(
        f.events("content.read")
            .iter()
            .all(|payload| payload["tool"] != "list_proposals")
    );
}

#[tokio::test]
async fn list_proposals_is_refused_under_every_revocation_level() {
    for (level, reason) in [
        (RevocationLevel::Instance, "instance_revoked"),
        (RevocationLevel::Type, "type_revoked"),
        (RevocationLevel::Global, "global_revoked"),
    ] {
        let f = fixture(true).await;
        let (session, instance) = f.initialize("claude").await;
        f.propose_rule("claude", &session, 2, "a.md", "# A\n").await;
        let target = match level {
            RevocationLevel::Instance => instance.as_str(),
            _ => "claude",
        };
        assert!(
            f.policy()
                .revoke(level, target, "owner", "测试", NOW)
                .unwrap()
        );
        // 每次调用都重验三级吊销：只读的列表也不例外，被拒不记读取审计。
        let refused = f
            .call("claude", &session, 3, "list_proposals", json!({}))
            .await;
        assert!(refused.get("result").is_none(), "{level:?}: {refused}");
        assert_eq!(f.denied_reasons().last().unwrap(), reason, "{level:?}");
        assert!(
            f.events("content.read")
                .iter()
                .all(|payload| payload["tool"] != "list_proposals"),
            "{level:?}"
        );
    }
}

#[tokio::test]
async fn list_proposals_without_a_policy_plane_is_refused_like_policy_read() {
    let f = fixture(false).await;
    let (session, _) = f.initialize("claude").await;
    let refused = f
        .call("claude", &session, 2, "list_proposals", json!({}))
        .await;
    assert!(refused.get("result").is_none(), "{refused}");
    assert_eq!(refused["error"]["code"], -32600);
    assert_eq!(refused["error"]["message"], "策略平面未接入（M3 兼容模式）");
    assert_eq!(f.denied_reasons().last().unwrap(), "policy_plane_absent");
}
