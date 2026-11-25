//! 连接器入口（ADR-046）的边界：从结果上看不出来、或要卡在一瞬间才看得出来的几条靠这里守。
//!
//! - agent 那一半不碰供给副本，也不借 MCP 那一侧的代码：`AppState` 只用得到五样，引入的东西逐个列名。
//! - 人侧账本只记两样：新会话第一次出现、限频的拒绝；更新和过期不进账本，先登记后落下。
//! - 人侧只读引擎的两个查询，不改任何工作项。
//! - 状态的纯逻辑不碰文件、网络和时钟。
//! - 连接器类型进不了 MCP 和 `/read`：在每次复核都要过的那一处判，只此一处。
//! - 排队、等锁期间撤了权就一项不给：交付前的再复核不能少。
//!
//! 按记号比（`rust_lexer`）：注释掉的、写进字符串的都不算数；挪位置、夹条件也算没有。
use crate::support::{
    rust_lexer::{lex, lex_product},
    workspace_root,
};
use std::{fs, path::Path};

const AGENT: [&str; 3] = [
    "crates/agent-server/src/connector.rs",
    "crates/agent-server/src/connector/limits.rs",
    "crates/agent-server/src/connector/reply.rs",
];
const HUMAN: &str = "crates/local-server/src/board/connector.rs";
const PRESENCE: &str = "crates/local-server/src/board/presence.rs";
const GRANT: &str = "crates/agent-server/src/server/grant.rs";

/// agent 那一半从 `AppState` 上只拿这五样；供给副本、白名单、会话都不在里面。
const STATE: [&str; 5] = ["audit", "board", "lifecycle", "policy", "store"];
/// agent 那一半从本 crate 和供给库引入的全部东西（原样的 `use` 段）。
const AGENT_USES: [&str; 3] = [
    "use crate::{ auth::{self, AgentType, CredentialDigest}, board_client::ConnectorError, \
     http_text::strict_query, shutdown::SERVER_STOPPING, state::{AppState, now}, };",
    "use studiolo_policy::CapabilitySet;",
    "use studiolo_supply::audit::{self, Actor};",
];
/// 人侧那一半要是活代码、各恰好一处的几段。
const HUMAN_ANCHORS: [&str; 4] = [
    // 先登记后落下，而且只有「这一份里第一次出现的」才登记。
    "for fresh in &planned.fresh { self.record_fresh(&call, fresh) \
     .map_err(|_| Failure::Unavailable)?; presence.note_record(&call.agent_type, now); }",
    "if !presence.denial_due(&call.agent_type, reason, now) { return; } \
     if let Ok(Some(_operation)) = self.audit.try_enter_operation() { let _ = self.record(json!({",
    // 交付前再复核。
    "projects.push(ProjectItems { project, items, finished, truncated, }); } \
     self.confirm(call, board_roster, &grant)?; Ok(",
    ".ok_or(Failure::NotFound)?; self.confirm(call, board_roster, &grant)?; value(view)",
];
/// agent 那一半交付前的再复核、先审计后给。
const AGENT_ANCHORS: [&str; 5] = [
    "self.recheck(caller, Entry::Board, &instance, &granted)?; let projects",
    "self.recheck(caller, Entry::Task, &instance, &granted)?; if reply",
    "self.recheck(caller, Entry::Sessions, &instance, &granted)?; Ok(Json(reply)",
    "for name in named { self.record(caller, &instance, &granted, name, Entry::Board)?; } \
     let mut response = Json(reply)",
    "self.record(caller, &instance, &granted, project, Entry::Task)?; Ok(Json(reply)",
];
const GRANT_ANCHOR: &str = "let grant = authorization.grant.ok_or_else(|| \
     McpError::internal_error(\"\", None))?; if grant.set.connector_only() { self.deny( \
     Actor::instance(&instance.agent_type, &instance.id), action, \"\", ); \
     return Err(McpError::invalid_request( format!( \"\" ), None, )); } let whitelist =";

fn source(path: &str) -> String {
    fs::read_to_string(workspace_root().join(path)).unwrap()
}

fn count(tokens: &[String], fragment: &str) -> usize {
    let fragment = lex(fragment);
    tokens
        .windows(fragment.len())
        .filter(|window| *window == fragment)
        .count()
}

/// `fragment` 之后紧跟着的那个记号，逐处列出。
fn followers<'a>(tokens: &'a [String], fragment: &str) -> Vec<&'a str> {
    let fragment = lex(fragment);
    tokens
        .windows(fragment.len() + 1)
        .filter(|window| window[..fragment.len()] == fragment)
        .map(|window| window[fragment.len()].as_str())
        .collect()
}

fn word(tokens: &[String], name: &str) -> usize {
    tokens.iter().filter(|token| *token == name).count()
}

/// agent 入口的三个文件合起来看。
fn agent_stays_narrow(sources: &[String]) -> bool {
    let all = sources
        .iter()
        .flat_map(|source| lex_product(source))
        .collect::<Vec<_>>();
    AGENT_USES.iter().all(|line| count(&all, line) == 1)
        && word(&all, "crate") == 1 + count(&all, "pub(crate)")
        && word(&all, "studiolo_policy") == 1
        && word(&all, "studiolo_supply") == 1
        // 换个名字拿走整个 `AppState` 就绕过了下面那条，所以只许 `.app.` 和建路由时的那几处。
        && followers(&all, "app").iter().all(|next| [".", ":", ","].contains(next))
        && count(&all, "app,") == 1
        && count(&all, "app.clone()") == 1
        && word(&all, "super") == count(&all, "pub(super)")
        && followers(&all, "app.")
            .iter()
            .all(|field| STATE.contains(field) || *field == "clone")
        && ["fs", "File", "OpenOptions"].iter().all(|name| word(&all, name) == 0)
        && AGENT_ANCHORS.iter().all(|anchor| count(&all, anchor) == 1)
}

fn human_only_reads(source: &str) -> bool {
    let tokens = lex_product(source);
    HUMAN_ANCHORS
        .iter()
        .all(|anchor| count(&tokens, anchor) == 1)
        && count(&tokens, "self.audit.append(") == 1
        && count(&tokens, ".append(") == 1
        && count(&tokens, "self.record(") == 2
        && count(&tokens, "self.record_fresh(") == 1
        && count(&tokens, "presence.commit(") == 1
        // 引擎只有读清单那一处借成变量，其余当场查完；两处都只许查询。
        && word(&tokens, "engine") == 4
        && count(&tokens, "engine.board_list(") == 1
        && count(&tokens, ".engine.lock().map_err(|_| Failure::Unavailable)?.board_detail(") == 1
}

fn presence_is_pure(source: &str) -> bool {
    let tokens = lex_product(source);
    count(
        &tokens,
        "use std::{ collections::{BTreeMap, BTreeSet, VecDeque}, time::Duration, };",
    ) == 1
        && word(&tokens, "std") == 1
        && word(&tokens, "super") == count(&tokens, "pub(super)")
        && word(&tokens, "extern") == 0
        && [
            "crate",
            "core",
            "alloc",
            "jiff",
            "Instant",
            "SystemTime",
            "tokio",
            "axum",
            "rustix",
            "libc",
            "rusqlite",
            "ulid",
            "include",
            "unsafe",
        ]
        .iter()
        .all(|name| word(&tokens, name) == 0)
}

fn refused_in_one_place(grant: &str, others: &[String]) -> bool {
    count(&lex_product(grant), GRANT_ANCHOR) == 1
        && word(&lex_product(grant), "connector_only") == 1
        && others
            .iter()
            .all(|source| word(&lex_product(source), "connector_only") == 0)
}

/// `crates/agent-server/src` 里 `grant.rs` 以外的全部源码。
fn other_agent_sources() -> Vec<String> {
    fn walk(directory: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !path.ends_with("server/grant.rs")
            {
                found.push(fs::read_to_string(path).unwrap());
            }
        }
    }
    let mut found = Vec::new();
    walk(
        &workspace_root().join("crates/agent-server/src"),
        &mut found,
    );
    assert!(found.len() > 20, "没找到 agent 的源码");
    found
}

#[test]
fn connector_entry_stays_inside_its_boundary() {
    // 换行、空白、注释不影响判断。
    for reformat in [
        (|text: String| text) as fn(String) -> String,
        |text: String| text.replace(";\n", "; // 说明\n\n"),
    ] {
        let agent = AGENT.map(|path| reformat(source(path)));
        assert!(agent_stays_narrow(&agent), "agent 入口");
        assert!(human_only_reads(&reformat(source(HUMAN))), "{HUMAN}");
        assert!(presence_is_pure(&reformat(source(PRESENCE))), "{PRESENCE}");
        assert!(
            refused_in_one_place(&reformat(source(GRANT)), &other_agent_sources()),
            "{GRANT}"
        );
    }
}

/// 在真源码上改错一处；改不动就是用例过时了。
fn changed(path: &str, before: &str, after: &str) -> String {
    let text = source(path);
    assert_eq!(text.matches(before).count(), 1, "{path}：{before}");
    text.replacen(before, after, 1)
}

#[test]
fn every_way_out_of_the_agent_boundary_is_caught() {
    for (file, before, after) in [
        (
            0,
            "state::{AppState, now},",
            "state::{AppState, now}, freshness,",
        ),
        (
            0,
            "use limits::Limits;",
            "use limits::Limits;\nuse crate::server::grant;",
        ),
        (
            2,
            "/// 每个答复都带的头",
            "use crate::state::AppState;\n/// 每个答复都带的头",
        ),
        (
            0,
            "audit::{self, Actor};",
            "{audit::{self, Actor}, ReadOnlyVault};",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let _ = &self.app.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let whole = &self.app; let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let whole = self.app.clone(); let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "use limits::Limits;",
            "use limits::Limits;\nuse super::server::grant;",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let AppState { allow, .. } = &*self.app;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "self.recheck(caller, Entry::Board, &instance, &granted)?;",
            "let _ = self.recheck(caller, Entry::Board, &instance, &granted);",
        ),
        (
            0,
            "self.recheck(caller, Entry::Task, &instance, &granted)?;",
            "// self.recheck(caller, Entry::Task, &instance, &granted)?;",
        ),
        (
            0,
            "self.recheck(caller, Entry::Sessions, &instance, &granted)?;",
            "#[cfg(test)]\n        self.recheck(caller, Entry::Board, &instance, &granted)?;",
        ),
        (
            0,
            "self.record(caller, &instance, &granted, name, Entry::Board)?;",
            "let _ = self.record(caller, &instance, &granted, name, Entry::Board);",
        ),
        (
            0,
            "self.record(caller, &instance, &granted, project, Entry::Task)?;",
            "",
        ),
    ] {
        let mut sources = AGENT.map(source);
        sources[file] = changed(AGENT[file], before, after);
        assert!(!agent_stays_narrow(&sources), "改成「{after}」没判红");
    }
}

#[test]
fn every_way_of_writing_more_on_the_human_side_is_caught() {
    for (before, after) in [
        // 每份快照都登记、逐条状态进账本。
        (
            "for fresh in &planned.fresh {",
            "for fresh in &planned.all {",
        ),
        (
            "presence.commit(&call.agent_type,",
            "let _ = self.record(json!({\"seq\": 1}));\n        presence.commit(&call.agent_type,",
        ),
        (
            "presence.commit(&call.agent_type,",
            "let _ = self.audit.append(event);\n        presence.commit(&call.agent_type,",
        ),
        // 登记不了也照收。
        (
            "self.record_fresh(&call, fresh)\n                    .map_err(|_| Failure::Unavailable)?;",
            "let _ = self.record_fresh(&call, fresh);",
        ),
        // 拒绝不限频。
        (
            "if !presence.denial_due(&call.agent_type, reason, now) {\n            return;\n        }",
            "",
        ),
        // 改工作项。
        (
            "let engine = self.engine.lock().map_err(|_| Failure::Unavailable)?;",
            "let engine = self.engine.lock().map_err(|_| Failure::Unavailable)?;\n            engine.board_cancel(1);",
        ),
        (
            ".board_detail(task_id, &grant.hash)",
            ".board_claim(task_id, &grant.hash)",
        ),
        // 交付前不再复核。
        (
            "self.confirm(call, board_roster, &grant)?;\n        Ok(json!({",
            "Ok(json!({",
        ),
        (
            "self.confirm(call, board_roster, &grant)?;\n        value(view)",
            "let _ = self.confirm(call, board_roster, &grant);\n        value(view)",
        ),
    ] {
        assert!(
            !human_only_reads(&changed(HUMAN, before, after)),
            "改成「{after}」没判红"
        );
    }
}

#[test]
fn presence_reaching_outside_is_caught() {
    for addition in [
        "fn later() { let _ = std::time::Instant::now(); }",
        "fn later() { let _ = std::fs::read(\"x\"); }",
        "use std::time::SystemTime;",
        "fn later() { let _ = jiff::Timestamp::now(); }",
        "use crate::editing::Problem;",
        "use super::BoardPlane;",
        "extern crate tokio as harmless;",
    ] {
        let text = changed(
            PRESENCE,
            "use studiolo_core::VaultPath;",
            &format!("use studiolo_core::VaultPath;\n{addition}"),
        );
        assert!(!presence_is_pure(&text), "加上「{addition}」没判红");
    }
}

#[test]
fn letting_connector_types_into_mcp_is_caught() {
    let others = other_agent_sources();
    for (before, after) in [
        (
            "if grant.set.connector_only() {",
            "if false && grant.set.connector_only() {",
        ),
        (
            "if grant.set.connector_only() {",
            "if !grant.set.connector_only() {",
        ),
        (
            "if grant.set.connector_only() {",
            "if grant.set.write.board {",
        ),
        (
            "        let whitelist = grant",
            "        if grant.set.connector_only() {}\n        let whitelist = grant",
        ),
    ] {
        assert!(
            !refused_in_one_place(&changed(GRANT, before, after), &others),
            "改成「{after}」没判红"
        );
    }
    // 别处另判一遍也算错：两处会走偏。
    let elsewhere = vec!["fn f() { if grant.set.connector_only() { return; } }".to_owned()];
    assert!(!refused_in_one_place(&source(GRANT), &elsewhere));
}
