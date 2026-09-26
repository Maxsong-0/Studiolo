//! ADR-026 规则级读权限（真实 loopback JSON-RPC）：能力集 `read.rules_whitelist` 对接入方的五条读取路径
//! ——`policy_read` 清单与单条、握手路牌（策略规则与库文件兜底）、`read_note`／`list_entries`／`search`
//! ——用同一个判定。名单外与不存在对外同一句，审计记 `rule_not_whitelisted`；`_规则/` 子目录照项目
//! 白名单（D9）；能力集缺字段即零规则（D4）；未接策略平面的 M3 兼容模式不做规则过滤（D10）。

use std::{fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_policy::{DesiredInput, DesiredStatus, PolicyKind, PolicyStore, Submitter};
use studiolo_supply::AgentStore;
use tempfile::TempDir;

const NOW: &str = "2026-09-26T23:00:00Z";

struct Fixture {
    _temp: TempDir,
    data: std::path::PathBuf,
    policy: std::path::PathBuf,
    token: String,
    base: String,
    read_base: String,
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

/// 与 `policy_contract.rs` 同一构造；`rules` 为 `None` 时正文里根本没有 `rules_whitelist` 字段。
fn capability(rules: Option<&[&str]>) -> String {
    let mut read = json!({"project_whitelist": ["项目/Alpha", "_规则"], "deny_by_default": true});
    if let Some(rules) = rules {
        read["rules_whitelist"] = json!(rules);
    }
    json!({
        "ai_type": "claude",
        "read": read,
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": false, "activate": false},
        "hard_bound_skills": [],
    })
    .to_string()
}

fn publish(store: &PolicyStore, kind: PolicyKind, subject: &str, body: &str) {
    let record = store
        .submit_desired(
            DesiredInput {
                kind,
                subject: subject.into(),
                body: Some(body.to_owned()),
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

/// 部署上限含 `项目/Alpha` 与 `_规则`；`with_policy` 为假即 M3 兼容模式（不给 `--policy`）。
async fn fixture(with_policy: bool) -> Fixture {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 状态 | 进行中 |\n",
    );
    write(&vault.join("_规则/A.md"), "# A\n\n共同词 A-VAULT\n");
    write(&vault.join("_规则/B.md"), "# B\n\n共同词 B-VAULT-CANARY\n");
    write(
        &vault.join("_规则/references/x.md"),
        "# 参考\n\n共同词 REF\n",
    );
    write(
        &vault.join("_规则/通用工作规则.md"),
        "# 规则\n\n## 准则① 名单外的母本准则\n\n共同词 MOTHER-CANARY\n",
    );
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));
    let store = AgentStore::open(&data).unwrap();
    let token = store.issue_type("claude", NOW).unwrap().token;
    drop(store);
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());
    let policy = temp.path().join("policy-data");
    private_dir(&policy);
    let store = PolicyStore::open(&policy).unwrap();
    publish(&store, PolicyKind::Rules, "A.md", "# A\n\nA-POLICY\n");
    publish(
        &store,
        PolicyKind::Rules,
        "B.md",
        "# B\n\n## 准则① 名单外的策略准则\n\nB-POLICY-CANARY\n",
    );
    drop(store);
    let state = serve::prepare(&ServeOptions {
        board_socket: None,
        index_cache: None,
        vault: Some(vault),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into(), "_规则".into()],
        data: data.clone(),
        policy: with_policy.then_some(policy.clone()),
        port: 0,
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = serve::router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fixture {
        _temp: temp,
        data,
        policy,
        token,
        base: format!("http://127.0.0.1:{}/mcp", addr.port()),
        read_base: format!("http://127.0.0.1:{}/read", addr.port()),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap(),
    }
}

impl Fixture {
    fn publish_capability(&self, rules: Option<&[&str]>) {
        let store = PolicyStore::open_existing(&self.policy).unwrap();
        publish(
            &store,
            PolicyKind::CapabilitySet,
            "claude",
            &capability(rules),
        );
    }

    fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }

    fn payload_field(&self, action: &str, field: &str) -> Vec<String> {
        self.ledger()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
            .iter()
            .map(|record| {
                record.event().payload().as_value()[field]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    fn last_denial(&self) -> String {
        self.payload_field("agent.action_denied", "reason")
            .pop()
            .unwrap()
    }

    async fn rpc(&self, session: Option<&str>, id: u64, method: &str, params: Value) -> Value {
        let mut request = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Authorization", format!("Bearer {}", self.token))
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
        let mut buffer = String::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
            // 应答可能是整段 JSON，也可能是 SSE 事件；读到目标 id 即停。
            let mut candidates = vec![buffer.clone()];
            candidates.extend(buffer.split("\n\n").map(|event| {
                event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim)
                    .collect::<String>()
            }));
            for data in candidates {
                if let Ok(mut value) = serde_json::from_str::<Value>(&data)
                    && value.get("id") == Some(&json!(id))
                {
                    if let Some(session) = &session_id {
                        value["session"] = json!(session);
                    }
                    return value;
                }
            }
        }
        panic!("no JSON-RPC response for {method}: {buffer}");
    }

    /// 握手并确认会话，返回（会话，路牌）。
    async fn initialize(&self) -> (String, String) {
        let response = self
            .rpc(
                None,
                1,
                "initialize",
                json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "rules", "version": "0"}}),
            )
            .await;
        let session = response["session"].as_str().unwrap().to_owned();
        let status = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Mcp-Session-Id", &session)
            .body(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 202);
        let signpost = response["result"]["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        (session, signpost)
    }

    async fn call(&self, session: &str, id: u64, name: &str, arguments: Value) -> Value {
        self.rpc(
            Some(session),
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
    }

    /// 成功调用的文本；失败即带响应断言。
    async fn text(&self, session: &str, id: u64, name: &str, arguments: Value) -> String {
        let response = self.call(session, id, name, arguments).await;
        assert!(
            response.get("error").is_none() && response["result"]["isError"] != json!(true),
            "{name}: {response}"
        );
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// 被拒调用的错误对象（code 与 message），并断言正文金丝雀没有漏出。
    async fn refused(&self, session: &str, id: u64, name: &str, arguments: Value) -> Value {
        let response = self.call(session, id, name, arguments).await;
        let error = response["error"].clone();
        assert!(error.is_object(), "{name} 应被拒绝：{response}");
        assert!(!response.to_string().contains("CANARY"), "{response}");
        error
    }
}

/// 清单输出在计数行（`… N 条`／`N 份规则母本`）之后的数据行；计数须与行数一致。
fn rows(text: &str, marker: &str) -> Vec<String> {
    let mut lines = text.lines();
    let head = lines.by_ref().find(|line| line.contains(marker)).unwrap();
    let rows = lines.map(str::to_owned).collect::<Vec<_>>();
    let count = head
        .split(marker)
        .next()
        .unwrap()
        .rsplit(' ')
        .next()
        .unwrap();
    assert_eq!(
        count.parse::<usize>().unwrap(),
        rows.len(),
        "计数与行数不一致：{text}"
    );
    rows
}

/// 每行按分隔符取第 `field` 列。
fn column(rows: &[String], separator: char, field: usize) -> Vec<String> {
    rows.iter()
        .map(|row| row.split(separator).nth(field).unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn rules_outside_the_whitelist_are_invisible_on_every_read_path() {
    let f = fixture(true).await;
    f.publish_capability(Some(&["A.md"]));
    let (session, signpost) = f.initialize().await;
    // 路牌：名单内的 A 进第 6 项入口；B 的准则与入口、名单外库文件的准则与入口都不出现。
    assert!(
        signpost.contains("入口：policy://rules/A.md@v"),
        "{signpost}"
    );
    for hidden in [
        "policy://rules/B.md",
        "名单外的策略准则",
        "名单外的母本准则",
        "_规则/通用工作规则.md",
        "_规则/B.md",
    ] {
        assert!(!signpost.contains(hidden), "{hidden} 漏进路牌：{signpost}");
    }

    // policy_read 清单只有 A，计数与行数一致；A 的正文可读。
    let listing = f
        .text(&session, 2, "policy_read", json!({"kind": "rules"}))
        .await;
    assert_eq!(column(&rows(&listing, " 份规则母本"), '\t', 0), ["A.md"]);
    let rule = f
        .text(
            &session,
            3,
            "policy_read",
            json!({"kind": "rules", "subject": "A.md"}),
        )
        .await;
    assert!(rule.contains("A-POLICY"));
    // 名单外：对外报「不存在」，审计记真实原因。
    let outside = f
        .refused(
            &session,
            4,
            "policy_read",
            json!({"kind": "rules", "subject": "B.md"}),
        )
        .await;
    assert_eq!(f.last_denial(), "rule_not_whitelisted");
    assert_eq!(outside["message"], "规则母本不存在");

    // read_note：名单内规则文件与 `_规则/` 子目录可读；名单外规则文件与白名单外路径同一句拒绝。
    let a = f
        .text(&session, 5, "read_note", json!({"path": "_规则/A.md"}))
        .await;
    assert!(a.contains("A-VAULT"));
    let reference = f
        .text(
            &session,
            6,
            "read_note",
            json!({"path": "_规则/references/x.md"}),
        )
        .await;
    assert!(reference.contains("REF"));
    let denied = f
        .refused(&session, 7, "read_note", json!({"path": "_规则/B.md"}))
        .await;
    assert_eq!(f.last_denial(), "rule_not_whitelisted");
    let elsewhere = f
        .refused(&session, 8, "read_note", json!({"path": "项目/Beta/x.md"}))
        .await;
    assert_eq!(f.last_denial(), "outside_allowlist");
    assert_eq!(denied, elsewhere);
    assert_eq!(denied["message"], "路径不在供给白名单内");

    // list_entries：名单外规则文件不进清单（计数随之一致），子目录照常列出。
    let entries = f
        .text(&session, 9, "list_entries", json!({"project": "_规则"}))
        .await;
    let paths = column(&rows(&entries, " 条"), '\t', 1);
    assert!(paths.contains(&"_规则/A.md".to_owned()), "{entries}");
    assert!(paths.contains(&"_规则/references/x.md".to_owned()));
    assert!(!paths.contains(&"_规则/B.md".to_owned()));
    assert!(!paths.contains(&"_规则/通用工作规则.md".to_owned()));

    // search：全文与路径两种模式都不命中名单外规则文件。
    let hits = f
        .text(
            &session,
            10,
            "search",
            json!({"project": "_规则", "text": "共同词"}),
        )
        .await;
    let hit_paths = column(&rows(&hits, " 条"), ':', 0);
    assert!(hit_paths.contains(&"_规则/A.md".to_owned()), "{hits}");
    assert!(hit_paths.contains(&"_规则/references/x.md".to_owned()));
    assert!(!hits.contains("_规则/B.md") && !hits.contains("通用工作规则"));
    let by_path = f
        .text(
            &session,
            11,
            "search",
            json!({"project": "_规则", "text": "B.md", "mode": "path"}),
        )
        .await;
    assert!(rows(&by_path, " 条").is_empty(), "{by_path}");

    // 权威账本：名单外的规则文件与规则母本一次也没有作为内容供给。
    let served = f.payload_field("content.read", "path");
    assert!(served.contains(&"policy://rules/A.md".to_owned()));
    for hidden in ["policy://rules/B.md", "_规则/B.md", "_规则/通用工作规则.md"] {
        assert!(!served.contains(&hidden.to_owned()), "{hidden}: {served:?}");
    }

    // 名单内但不存在的规则：与名单外同一个对外错误，审计记 not_found。
    f.publish_capability(Some(&["A.md", "Z.md"]));
    let absent = f
        .refused(
            &session,
            12,
            "policy_read",
            json!({"kind": "rules", "subject": "Z.md"}),
        )
        .await;
    assert_eq!(f.last_denial(), "not_found");
    assert_eq!(absent, outside);

    // HTTP 文本外壳走同一判定：名单外与白名单外同为 400，与不存在同为 404。
    let handshake = f
        .client
        .get(format!("{}/handshake", f.read_base))
        .header("Authorization", format!("Bearer {}", f.token))
        .send()
        .await
        .unwrap();
    assert_eq!(handshake.status(), 200);
    let http_session = handshake
        .headers()
        .get("studiolo-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        !handshake
            .text()
            .await
            .unwrap()
            .contains("policy://rules/B.md")
    );
    let mut bodies = Vec::new();
    for (query, status) in [
        ("note?path=_规则/B.md", 400),
        ("note?path=项目/Beta/x.md", 400),
        ("policy?kind=rules&subject=B.md", 404),
        ("policy?kind=rules&subject=Z.md", 404),
    ] {
        let response = f
            .client
            .get(format!("{}/{query}", f.read_base))
            .header("Authorization", format!("Bearer {}", f.token))
            .header("Studiolo-Session-Id", &http_session)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{query}");
        bodies.push(response.text().await.unwrap());
    }
    assert!(bodies.iter().all(|body| !body.contains("CANARY")));
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[2], bodies[3]);
}

#[tokio::test]
async fn a_capability_set_without_the_field_reads_no_rules() {
    let f = fixture(true).await;
    f.publish_capability(None);
    let (session, signpost) = f.initialize().await;
    assert!(!signpost.contains("policy://rules/"), "{signpost}");
    assert!(!signpost.contains("_规则/通用工作规则.md"));
    assert!(signpost.contains("不可用（白名单内无规则母本）"));
    let listing = f
        .text(&session, 2, "policy_read", json!({"kind": "rules"}))
        .await;
    assert!(rows(&listing, " 份规则母本").is_empty(), "{listing}");
    f.refused(
        &session,
        3,
        "policy_read",
        json!({"kind": "rules", "subject": "A.md"}),
    )
    .await;
    assert_eq!(f.last_denial(), "rule_not_whitelisted");
    f.refused(&session, 4, "read_note", json!({"path": "_规则/A.md"}))
        .await;
    assert_eq!(f.last_denial(), "rule_not_whitelisted");
    // `_规则/` 子目录不是规则文件，照项目白名单可读（D9）。
    f.text(
        &session,
        5,
        "read_note",
        json!({"path": "_规则/references/x.md"}),
    )
    .await;
    let entries = f
        .text(&session, 6, "list_entries", json!({"project": "_规则"}))
        .await;
    let paths = column(&rows(&entries, " 条"), '\t', 1);
    assert!(
        paths
            .iter()
            .all(|path| path.starts_with("_规则/references")),
        "{entries}"
    );
    let hits = f
        .text(
            &session,
            7,
            "search",
            json!({"project": "_规则", "text": "共同词"}),
        )
        .await;
    assert_eq!(
        column(&rows(&hits, " 条"), ':', 0),
        ["_规则/references/x.md"],
        "{hits}"
    );
}

#[tokio::test]
async fn m3_compatibility_mode_does_not_filter_rule_files() {
    let f = fixture(false).await;
    let (session, signpost) = f.initialize().await;
    assert!(signpost.contains("策略平面：未接入"));
    assert!(
        signpost.contains("入口：_规则/通用工作规则.md@"),
        "{signpost}"
    );
    let b = f
        .text(&session, 2, "read_note", json!({"path": "_规则/B.md"}))
        .await;
    assert!(b.contains("B-VAULT-CANARY"));
    let entries = f
        .text(&session, 3, "list_entries", json!({"project": "_规则"}))
        .await;
    assert!(entries.contains("_规则/B.md") && entries.contains("_规则/通用工作规则.md"));
    let hits = f
        .text(
            &session,
            4,
            "search",
            json!({"project": "_规则", "text": "共同词"}),
        )
        .await;
    assert!(hits.contains("_规则/B.md"), "{hits}");
}
