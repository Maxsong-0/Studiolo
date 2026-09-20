//! M4 接入方侧契约（真实 loopback JSON-RPC）：无能力集零权限、供给视图＝部署上限∩能力集、
//! 路牌第 3／5 项改自策略平面、policy_read 进读取审计、propose_desired 永不生效、
//! 三级吊销即刻生效与恢复方向、handshake.served 记 effective 哈希、HTTP 外壳同一授权。

use std::{fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, time::Duration};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_policy::{
    DesiredInput, DesiredStatus, PolicyKind, PolicyStore, RevocationLevel, Submitter,
};
use studiolo_supply::{AgentStore, HARD_BUDGET_TOKENS, ReadQuery, count_tokens};
use tempfile::TempDir;

const NOW: &str = "2026-09-17T23:00:00Z";

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

fn capability(whitelist: &[&str], bindings: &[(&str, &str)], propose: bool) -> String {
    json!({
        "ai_type": "claude",
        "read": {"project_whitelist": whitelist, "deny_by_default": true},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": propose, "activate": false},
        "hard_bound_skills": bindings.iter().map(|(when, skill)| json!({"when": when, "must_use": skill})).collect::<Vec<_>>(),
    })
    .to_string()
}

fn publish(store: &PolicyStore, kind: PolicyKind, subject: &str, body: Option<&str>) -> u64 {
    let record = store
        .submit_desired(
            DesiredInput {
                kind,
                subject: subject.into(),
                body: body.map(str::to_owned),
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
    store
        .activate(&record.id, &hash, "owner", NOW)
        .unwrap()
        .version
}

async fn fixture() -> Fixture {
    fixture_with_board(None).await
}
async fn fixture_with_board(board_socket: Option<std::path::PathBuf>) -> Fixture {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 🟡 进行中 |\n| 最近日志 | [[2026-09-06]] |\n| 未关闭 BUG | 无 |\n\n## 当前重点\n\n- **T-069 M4**：策略平面。\n",
    );
    write(
        &vault.join("项目/Alpha/02-日志/2026-09-06.md"),
        "# 日志\n\n做了 Alpha 的事。\n",
    );
    write(
        &vault.join("项目/Beta/00-总览/INDEX.md"),
        "# Beta · 总览\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v9 |\n| 状态 | 静默 |\n| 最近日志 | 无 |\n| 未关闭 BUG | 无 |\n",
    );
    write(
        &vault.join("_规则/通用工作规则.md"),
        "# 规则\n\n## 准则① vault 文件里的准则\n\n正文。\n\n**写 React 一律叠加 `vault-skill`。**\n",
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
    drop(PolicyStore::open(&policy).unwrap());

    let state = serve::prepare(&ServeOptions {
        board_socket,
        index_cache: None,
        vault: Some(vault.clone()),
        replica_root: None,
        replica_max_age_secs: 300,
        // 部署上限给三个目录；能力集再收窄。
        allow: vec!["项目/Alpha".into(), "项目/Beta".into(), "_规则".into()],
        data: data.clone(),
        policy: Some(policy.clone()),
        port: 0,
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = serve::router(state.clone());
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

#[tokio::test]
async fn board_response_is_withheld_when_its_project_is_removed_while_waiting() {
    use std::sync::Arc;
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let socket = temp.path().join("board.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let reached = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
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
                axum::Json(json!({"private":"WITHDRAWN_BOARD_CANARY"}))
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let f = Arc::new(fixture_with_board(Some(socket)).await);
    publish(
        &f.policy(),
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Alpha"], &[], true)),
    );
    let (session, _) = f.initialize().await.unwrap();
    let caller = f.clone();
    let pending = tokio::spawn(async move {
        caller
            .call(
                &session,
                2,
                "board",
                json!({"project":"项目/Alpha","operation":"get","request":{"task_id":"fixture"}}),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), reached.notified())
        .await
        .unwrap();
    publish(
        &f.policy(),
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Beta"], &[], true)),
    );
    release.notify_one();
    let response = pending.await.unwrap();
    assert!(response.get("error").is_some(), "{response}");
    assert!(!response.to_string().contains("WITHDRAWN_BOARD_CANARY"));
    server.abort();
}

impl Fixture {
    fn policy(&self) -> PolicyStore {
        PolicyStore::open_existing(&self.policy).unwrap()
    }

    fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.data.join("audit")).unwrap()
    }

    fn store(&self) -> AgentStore {
        AgentStore::open_existing(&self.data).unwrap()
    }

    fn request(&self, session: Option<&str>, body: Value) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.base)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", "2025-11-25")
            .header("Authorization", format!("Bearer {}", self.token))
            .body(body.to_string());
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
    }

    async fn rpc(
        &self,
        session: Option<&str>,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Option<String>, Value) {
        let mut response = self
            .request(
                session,
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            )
            .send()
            .await
            .unwrap();
        if response.status() != 200 {
            let status = response.status();
            let body = response.text().await.unwrap();
            panic!("{method}: HTTP {status}: {body}");
        }
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

    /// 握手；成功返回 (会话, 路牌)，失败返回错误对象。
    async fn initialize(&self) -> Result<(String, String), Value> {
        self.initialize_with_parent(None).await
    }
    async fn initialize_with_parent(
        &self,
        parent: Option<&str>,
    ) -> Result<(String, String), Value> {
        let mut params = json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "contract", "version": "0"}});
        if let Some(parent) = parent {
            params["_meta"] = json!({"studiolo.parent_instance":parent});
        }
        let (session, response) = self.rpc(None, 1, "initialize", params).await;
        if response.get("error").is_some() {
            return Err(response);
        }
        let session = session.expect("session id header");
        let status = self
            .request(
                Some(&session),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 202);
        let instructions = response["result"]["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        Ok((session, instructions))
    }

    async fn call(&self, session: &str, id: u64, name: &str, arguments: Value) -> Value {
        self.rpc(
            Some(session),
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )
        .await
        .1
    }

    fn denied_reasons(&self) -> Vec<String> {
        self.ledger()
            .query(&AuditFilter::default().with_action("agent.action_denied"))
            .unwrap()
            .iter()
            .map(|record| {
                record.event().payload().as_value()["reason"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    fn instance_of(instructions: &str) -> String {
        instructions
            .split("实例 ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .to_owned()
    }
}

#[tokio::test]
async fn registered_child_metadata_and_ancestor_revocation_are_enforced_in_mcp() {
    let f = fixture().await;
    f.store().upgrade_child_instances().unwrap();
    let policy = f.policy();
    publish(
        &policy,
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Alpha"], &[], true)),
    );
    let (_, parent) = f.initialize().await.unwrap();
    let parent_id = Fixture::instance_of(&parent);
    let (child_session, child) = f.initialize_with_parent(Some(&parent_id)).await.unwrap();
    let child_id = Fixture::instance_of(&child);
    let chain = f.store().instance_lineage(&child_id).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[1].id, parent_id);
    policy
        .revoke(
            RevocationLevel::Instance,
            &parent_id,
            "fixture revoke",
            "owner",
            NOW,
        )
        .unwrap();
    let denied = f
        .call(
            &child_session,
            2,
            "list_entries",
            json!({"project":"项目/Alpha"}),
        )
        .await;
    assert!(denied.get("error").is_some(), "{denied}");
    let forbidden = f.initialize_with_parent(Some(&parent_id)).await;
    assert!(forbidden.is_err() || forbidden.is_ok_and(|(_, text)| text.starts_with("策略拒绝：")));
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

#[tokio::test]
async fn a_type_without_a_capability_set_has_zero_rights_until_one_is_published() {
    let f = fixture().await;
    // RG-CONSUMER 1：有 token、无能力集 → 握手「应答但一项不给」：instructions 只有拒绝原因，
    // 没有路牌六项，也没有任何 vault 内容；审计 no_capability_set；后续调用照样拒绝。
    let (zero, instructions) = f.initialize().await.unwrap();
    assert!(
        instructions.starts_with("策略拒绝：") && instructions.contains("零权限"),
        "{instructions}"
    );
    for leaked in ["# Studiolo 路牌", "Alpha", "## 1 我是谁", "入口："] {
        assert!(
            !instructions.contains(leaked),
            "{leaked} leaked: {instructions}"
        );
    }
    assert_eq!(f.denied_reasons(), ["no_capability_set"]);
    let still = f
        .call(
            &zero,
            2,
            "read_note",
            json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(is_error(&still) && !still.to_string().contains("Alpha · 总览"));
    assert_eq!(f.denied_reasons().last().unwrap(), "no_capability_set");
    // HTTP 外壳同一授权：403 而不是 401，也不提示重新握手。
    let response = f
        .client
        .get(format!("{}/handshake", f.read_base))
        .header("Authorization", format!("Bearer {}", f.token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(response.text().await.unwrap().contains("零权限"));

    // 发布能力集：只开 项目/Alpha，部署上限里的 Beta 与 _规则 都不给。
    let store = f.policy();
    publish(
        &store,
        PolicyKind::Skills,
        "vercel-react-best-practices",
        Some(
            &json!({"name": "vercel-react-best-practices", "summary": "React 性能规范", "triggers": ["写或改 React / Next.js 代码"], "markdown": "# 规范\n用 RSC。\n"}).to_string(),
        ),
    );
    publish(
        &store,
        PolicyKind::Rules,
        "通用工作规则.md",
        Some("# 规则\n\n## 准则① 策略平面里的准则\n\n正文。\n"),
    );
    let version = publish(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(
            &["项目/Alpha"],
            &[("写或改 React / Next.js 代码", "vercel-react-best-practices")],
            true,
        )),
    );
    let effective = store.effective().unwrap().effective_hash;

    let (session, instructions) = f.initialize().await.unwrap();
    assert!(count_tokens(&instructions) <= HARD_BUDGET_TOKENS);
    // 第 3 项：供给视图＝部署上限∩能力集，且带策略版本。
    assert!(
        instructions.contains(&format!(
            "读：项目/Alpha（能力集 v{version} ∩ 部署上限）；写：黑板 只读、正文 仅经预览的 patch；策略：可提 desired（永不自动生效）；策略版本 {}",
            &effective[..12]
        )),
        "{instructions}"
    );
    // 第 2 项只列供给视图内的项目。
    assert!(instructions.contains("### 项目/Alpha") && !instructions.contains("Beta"));
    // 第 5 项：准则来自策略平面而不是 vault 文件；硬绑定标「策略强制」；vault 里的句子不再出现。
    assert!(
        instructions.contains("1 策略平面里的准则"),
        "{instructions}"
    );
    assert!(!instructions.contains("vault 文件里的准则"));
    assert!(
        instructions.contains(
            "硬绑定（策略强制）：写或改 React / Next.js 代码 → vercel-react-best-practices"
        )
    );
    assert!(!instructions.contains("vault-skill"));
    // 第 6 项：策略入口。
    assert!(instructions.contains("入口：policy://rules/通用工作规则.md@v"));
    assert!(instructions.contains("skills：1 个"));
    assert!(instructions.contains("policy_read") && instructions.contains("propose_desired"));

    // handshake.served 记录了 effective 哈希，与策略库当前值一致（2.4 第 7 条）。
    let served = f
        .ledger()
        .query(&AuditFilter::default().with_action("handshake.served"))
        .unwrap();
    let payload = served.last().unwrap().event().payload().as_value();
    assert_eq!(payload["policy_plane"], "present");
    assert_eq!(payload["effective_hash"], effective);

    // 供给视图之外（部署上限之内）的目录：从一开始不进入计算。
    for (name, args) in [
        ("read_note", json!({"path": "项目/Beta/00-总览/INDEX.md"})),
        ("read_note", json!({"path": "_规则/通用工作规则.md"})),
        ("list_entries", json!({"project": "_规则"})),
        ("search", json!({"project": "项目/Beta", "text": "v9"})),
    ] {
        let denied = f.call(&session, 10, name, args).await;
        assert!(is_error(&denied), "{name}: {denied}");
        assert!(!denied.to_string().contains("v9"));
        assert_eq!(f.denied_reasons().last().unwrap(), "outside_allowlist");
    }
    let ok = f
        .call(
            &session,
            11,
            "read_note",
            json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
        )
        .await;
    assert!(!is_error(&ok));
    let listed = f
        .call(
            &session,
            12,
            "list_entries",
            json!({"project": "项目/Alpha"}),
        )
        .await;
    assert!(tool_text(&listed).contains("项目/Alpha/00-总览/INDEX.md"));

    // tools/list 含两个新 tool。
    let tools = f.rpc(Some(&session), 13, "tools/list", json!({})).await.1;
    let mut names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "board",
            "handshake",
            "list_entries",
            "policy_read",
            "propose_desired",
            "read_note",
            "search"
        ]
    );
}

#[tokio::test]
async fn policy_read_is_audited_and_proposals_never_change_effective() {
    let f = fixture().await;
    let store = f.policy();
    publish(
        &store,
        PolicyKind::Rules,
        "通用工作规则.md",
        Some("# 规则\n\n## 准则① 永远说中文\n\n禁止绕过式修复。\n"),
    );
    publish(
        &store,
        PolicyKind::Skills,
        "tdd",
        Some(&json!({"name": "tdd", "summary": "测试先行", "triggers": ["新功能"], "markdown": "# TDD\n红绿重构。\n"}).to_string()),
    );
    publish(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Alpha"], &[], true)),
    );
    let (session, instructions) = f.initialize().await.unwrap();
    let instance = Fixture::instance_of(&instructions);

    let rules = f
        .call(&session, 2, "policy_read", json!({"kind": "rules"}))
        .await;
    let text = tool_text(&rules);
    assert!(text.starts_with("# 策略版本 "));
    assert!(text.contains("通用工作规则.md\tv1\t"), "{text}");
    let rule = f
        .call(
            &session,
            3,
            "policy_read",
            json!({"kind": "rules", "subject": "通用工作规则.md"}),
        )
        .await;
    let text = tool_text(&rule);
    assert!(text.contains("policy://rules/通用工作规则.md@v1 sha256 "));
    assert!(text.contains("禁止绕过式修复。"));
    let skills = f
        .call(&session, 4, "policy_read", json!({"kind": "skills"}))
        .await;
    assert!(tool_text(&skills).contains("tdd\tv2\t测试先行\t新功能"));
    let skill = f
        .call(
            &session,
            5,
            "policy_read",
            json!({"kind": "skills", "subject": "tdd"}),
        )
        .await;
    assert!(tool_text(&skill).contains("红绿重构"));
    let missing = f
        .call(
            &session,
            6,
            "policy_read",
            json!({"kind": "rules", "subject": "没有.md"}),
        )
        .await;
    assert!(is_error(&missing));
    assert!(is_error(
        &f.call(
            &session,
            7,
            "policy_read",
            json!({"kind": "capability_set"})
        )
        .await
    ));
    // 每次策略读取都进读取审计与四维索引：policy:// 路径、v 版本号、tool=policy_read。
    let reads = f
        .store()
        .query_reads(&ReadQuery {
            instance_id: Some(instance.clone()),
            path_prefix: Some("policy://".into()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    let mut paths = reads
        .iter()
        .map(|record| {
            (
                record.path.clone(),
                record.revision.clone(),
                record.tool.clone(),
            )
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert_eq!(
        paths,
        [
            (
                "policy://rules".to_owned(),
                "-".to_owned(),
                "policy_read".to_owned()
            ),
            (
                "policy://rules/通用工作规则.md".into(),
                "v1".into(),
                "policy_read".into()
            ),
            ("policy://skills".into(), "-".into(), "policy_read".into()),
            (
                "policy://skills/tdd".into(),
                "v2".into(),
                "policy_read".into()
            ),
        ]
    );
    // HTTP 外壳同一入口。
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
    let via_http = f
        .client
        .get(format!("{}/policy?kind=skills&subject=tdd", f.read_base))
        .header("Authorization", format!("Bearer {}", f.token))
        .header("Studiolo-Session-Id", &http_session)
        .send()
        .await
        .unwrap();
    assert_eq!(via_http.status(), 200);
    assert!(via_http.text().await.unwrap().contains("红绿重构"));

    // 用例 36：AI 提交 desired 永不自动生效。
    let before = store.effective().unwrap().effective_hash;
    let proposed = f
        .call(
            &session,
            8,
            "propose_desired",
            json!({"kind": "capability_set", "subject": "claude", "body": capability(&["项目/Alpha", "项目/Beta"], &[], true)}),
        )
        .await;
    assert!(!is_error(&proposed), "{proposed}");
    let text = tool_text(&proposed);
    assert!(
        text.contains("status pending")
            && text.contains("ok true")
            && text.contains("expansions 1")
    );
    assert!(text.contains("永不自动生效"));
    let desired_id = text
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("desired_id ")
        .to_owned();
    assert_eq!(store.effective().unwrap().effective_hash, before);
    let record = store.desired(&desired_id).unwrap().unwrap();
    assert_eq!(record.status, DesiredStatus::Pending);
    assert_eq!(record.agent_instance.as_deref(), Some(instance.as_str()));
    let submitted = f
        .ledger()
        .query(&AuditFilter::default().with_action("policy.desired_submitted"))
        .unwrap();
    assert_eq!(submitted.len(), 1);
    let payload = submitted[0].event().payload().as_value();
    assert_eq!(payload["actor"], "agent");
    assert_eq!(payload["agent_instance"], instance);
    assert_eq!(payload["desired_id"], desired_id);
    assert!(payload.get("body").is_none());
    assert!(
        f.ledger()
            .query(&AuditFilter::default().with_action("policy.activated"))
            .unwrap()
            .is_empty()
    );
    // Beta 仍然读不到：提案没有扩大任何权限。
    assert!(is_error(
        &f.call(
            &session,
            9,
            "read_note",
            json!({"path": "项目/Beta/00-总览/INDEX.md"})
        )
        .await
    ));
    // 结构无效与非法 kind。
    let bad = f
        .call(
            &session,
            10,
            "propose_desired",
            json!({"kind": "capability_set", "subject": "claude", "body": "{}"}),
        )
        .await;
    assert!(tool_text(&bad).contains("status rejected"), "{bad}");
    assert!(is_error(
        &f.call(
            &session,
            11,
            "propose_desired",
            json!({"kind": "nope", "subject": "x", "body": "{}"})
        )
        .await
    ));
    // 没有提案能力的类型：capability_denied。
    let hash = store.effective().unwrap().effective_hash;
    let narrowed = store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::CapabilitySet,
                subject: "claude".into(),
                body: Some(capability(&["项目/Alpha"], &[], false)),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap();
    store.activate(&narrowed.id, &hash, "owner", NOW).unwrap();
    let denied = f
        .call(
            &session,
            12,
            "propose_desired",
            json!({"kind": "rules", "subject": "x.md", "body": "# x\n"}),
        )
        .await;
    assert!(is_error(&denied));
    assert_eq!(f.denied_reasons().last().unwrap(), "capability_denied");
}

#[tokio::test]
async fn three_level_revocation_is_immediate_for_running_sessions() {
    let f = fixture().await;
    let store = f.policy();
    publish(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Alpha"], &[], false)),
    );
    let (first, first_signpost) = f.initialize().await.unwrap();
    let (second, second_signpost) = f.initialize().await.unwrap();
    let first_instance = Fixture::instance_of(&first_signpost);
    let second_instance = Fixture::instance_of(&second_signpost);
    assert_ne!(first_instance, second_instance);
    let read = |session: String, id: u64| {
        let f = &f;
        async move {
            f.call(
                &session,
                id,
                "read_note",
                json!({"path": "项目/Alpha/00-总览/INDEX.md"}),
            )
            .await
        }
    };
    assert!(!is_error(&read(first.clone(), 2).await));

    // 用例 37：实例吊销后该实例即刻失效，兄弟实例不受影响。
    assert!(
        store
            .revoke(
                RevocationLevel::Instance,
                &first_instance,
                "owner",
                "测试",
                NOW
            )
            .unwrap()
    );
    let denied = read(first.clone(), 3).await;
    assert!(is_error(&denied));
    assert!(
        denied["error"]["message"]
            .as_str()
            .unwrap()
            .contains("实例已被吊销")
    );
    assert_eq!(f.denied_reasons().last().unwrap(), "instance_revoked");
    assert!(!is_error(&read(second.clone(), 4).await));

    // 类型吊销：全部实例即刻失效，新握手也拒绝。
    assert!(
        store
            .revoke(RevocationLevel::Type, "claude", "owner", "测试", NOW)
            .unwrap()
    );
    assert!(is_error(&read(second.clone(), 5).await));
    assert_eq!(f.denied_reasons().last().unwrap(), "type_revoked");
    let (_, refused) = f.initialize().await.unwrap();
    assert!(refused.starts_with("策略拒绝：类型已被吊销"), "{refused}");
    // 恢复方向只走发布：重新发布能力集即解除类型吊销；实例吊销不随之恢复。
    let restored = publish(
        &store,
        PolicyKind::CapabilitySet,
        "claude",
        Some(&capability(&["项目/Alpha"], &[], false)),
    );
    assert!(restored > 1);
    assert!(!is_error(&read(second.clone(), 6).await));
    assert!(is_error(&read(first.clone(), 7).await));

    // 全局吊销 → 全部拒绝；解除后恢复。
    assert!(
        store
            .revoke(RevocationLevel::Global, "*", "owner", "紧急", NOW)
            .unwrap()
    );
    assert!(is_error(&read(second.clone(), 8).await));
    assert_eq!(f.denied_reasons().last().unwrap(), "global_revoked");
    assert!(
        f.initialize()
            .await
            .unwrap()
            .1
            .starts_with("策略拒绝：全局吊销生效中")
    );
    store.release_global("owner", NOW).unwrap();
    assert!(!is_error(&read(second.clone(), 9).await));
    assert!(
        f.initialize()
            .await
            .unwrap()
            .1
            .starts_with("# Studiolo 路牌")
    );

    // 身份库的类型吊销（admin type-revoke 路径）依旧即刻生效：门禁处直接 401，两条 engage 路径并存。
    assert!(f.store().revoke_type("claude", NOW).unwrap());
    let stale = f
        .request(
            Some(&second),
            json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "read_note", "arguments": {"path": "项目/Alpha/00-总览/INDEX.md"}}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 401);
}

#[tokio::test]
async fn serve_without_policy_keeps_m3_behaviour_and_marks_the_plane_absent() {
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let vault = temp.path().join("vault");
    write(
        &vault.join("项目/Alpha/00-总览/INDEX.md"),
        "# Alpha\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | v0.3 |\n| 状态 | 进行中 |\n| 最近日志 | 无 |\n| 未关闭 BUG | 无 |\n",
    );
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));
    let token = AgentStore::open(&data)
        .unwrap()
        .issue_type("claude", NOW)
        .unwrap()
        .token;
    drop(SqliteAuditLedger::open(&data.join("audit")).unwrap());
    let state = serve::prepare(&ServeOptions {
        board_socket: None,
        index_cache: None,
        vault: Some(vault),
        replica_root: None,
        replica_max_age_secs: 300,
        allow: vec!["项目/Alpha".into()],
        data: data.clone(),
        policy: None,
        port: 0,
    })
    .unwrap();
    assert!(state.policy.is_none());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = serve::router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:{}/read/handshake", addr.port()))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let session = response
        .headers()
        .get("studiolo-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let text = response.text().await.unwrap();
    assert!(text.contains("只读；白名单：项目/Alpha；写与审批：无（M3）；策略平面：未接入"));
    let policy = client
        .get(format!(
            "http://127.0.0.1:{}/read/policy?kind=rules",
            addr.port()
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Studiolo-Session-Id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(policy.status(), 403);

    // MCP 侧同一语义。上线探针 `scripts/check-agent.py` 逐字依赖这里钉住的三点：
    // 路牌里的「策略平面：未接入」、tools/list 恒为六个、`policy_read` 的固定拒绝码与文案。
    // 改动其中任何一处都必须同步改探针，否则 4401 阶段验收会在上线当天失败（BUG-010）。
    let mcp = Fixture {
        _temp: temp,
        data: data.clone(),
        policy: data.join("absent"),
        token: token.clone(),
        base: format!("http://127.0.0.1:{}/mcp", addr.port()),
        read_base: format!("http://127.0.0.1:{}/read", addr.port()),
        client: client.clone(),
    };
    let (mcp_session, signpost) = mcp.initialize().await.unwrap();
    assert!(signpost.contains("策略平面：未接入"));
    let (_, listed) = mcp
        .rpc(Some(&mcp_session), 2, "tools/list", json!({}))
        .await;
    let mut names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "board",
            "handshake",
            "list_entries",
            "policy_read",
            "propose_desired",
            "read_note",
            "search"
        ]
    );
    let refused = mcp
        .call(&mcp_session, 3, "policy_read", json!({"kind": "skills"}))
        .await;
    assert!(refused.get("result").is_none());
    assert_eq!(refused["error"]["code"], -32600);
    assert_eq!(refused["error"]["message"], "策略平面未接入（M3 兼容模式）");

    let served = SqliteAuditLedger::open_existing(&data.join("audit"))
        .unwrap()
        .query(&AuditFilter::default().with_action("handshake.served"))
        .unwrap();
    assert_eq!(served.len(), 2, "HTTP 与 MCP 各握手一次");
    for record in &served {
        let payload = record.event().payload().as_value();
        assert_eq!(payload["policy_plane"], "absent");
        assert!(payload["effective_hash"].is_null());
    }
}
