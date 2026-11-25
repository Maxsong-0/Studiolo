//! 连接器入口（ADR-046）的边界：从结果上看不出来、或要卡在一瞬间才看得出来的几条靠这里守。
//!
//! - agent 那一半不碰供给副本，也不借 MCP 那一侧的代码：`AppState` 只用得到六样，引入的东西、文件逐个列名。
//! - agent 先收完正文、排上名额、取到操作准入，才碰身份库和策略库。
//! - 人侧账本只记两样：新会话第一次出现、限频的拒绝；更新和过期不进账本，先登记后落下。
//! - 人侧只读引擎的两个查询，不改任何工作项；`BoardPlane` 上用得到的逐个列名。
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

/// agent 那一半从 `AppState` 上只拿这六样；供给副本、白名单、会话都不在里面。
const STATE: [&str; 6] = [
    "audit",
    "begin_operation",
    "board",
    "lifecycle",
    "policy",
    "store",
];
/// 入口的子目录里只有这两个文件：守卫只看得到列名的。
const AGENT_FILES: [&str; 2] = ["limits.rs", "reply.rs"];
/// 状态那一侧的子目录里只有用例。
const PRESENCE_FILES: [&str; 1] = ["tests.rs"];
/// agent 那一半从本 crate 和供给库引入的全部东西（原样的 `use` 段）。
const AGENT_USES: [&str; 3] = [
    "use crate::{ auth::{self, AgentType, CredentialDigest, Unrecognized}, board_client::ConnectorError, \
     http_text::strict_query, shutdown::SERVER_STOPPING, state::{AppState, now}, };",
    "use studiolo_policy::CapabilitySet;",
    "use studiolo_supply::audit::{self, Actor};",
];
/// 人侧那一半从本模块引入的全部东西（原样的 `use` 段）。
const HUMAN_USES: &str = "use super::{ BoardPlane, auth::{Grant, IDENTITY_UNAVAILABLE, Refusal}, \
     execute::{Failure as BoardFailure, parse, value}, http::{busy, unavailable, unconfigured}, \
     overview::{Item, Lineages, terminal}, \
     presence::{EXPIRES, Fresh, Origin, Presence, Reason, Refused, Snapshot}, \
     requests::{self, Task}, };";
/// 人侧那一半能在 `BoardPlane`（和状态口自己）上用的：库句柄、授权、状态口和本文件里的方法。
/// 写工作项的 `commit`、`project` 这类现成方法不在里面。
const HUMAN_SELF: [&str; 18] = [
    "audit",
    "confirm",
    "connector_board",
    "connector_grant",
    "connector_sessions",
    "connector_task",
    "engine",
    "granted",
    "identities",
    "issued_at",
    "note_denial",
    "presence",
    "record",
    "record_fresh",
    "refresh_presence",
    "rostered",
    "state",
    "storage",
];
/// 用在 `self.` 之外的：读状态那一个。
const HUMAN_VIEW: &str = "presence_view";
/// 入口处理函数手里的 `plane` 能用的。
const HUMAN_PLANE: [&str; 4] = ["connect", "permits", "presence", "presence_view"];
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
/// agent 那一半：正文先收完再占名额，排上名额、取到操作准入才裁；交付前再复核、先审计后给；没认出的拒绝限频。
const AGENT_ANCHORS: [&str; 8] = [
    "Entry::Sessions => match read(body).await { Ok(body) => body, \
     Err(refusal) => return refusal.into_response(), }, }; \
     let Ok(_permit) = state.requests.try_acquire() else { \
     return TOO_MANY.after(5).into_response(); }; \
     let Ok(_slot) = state.authority.acquire().await else { \
     return UNAVAILABLE.into_response(); }; \
     let Some(_operation) = state.app.begin_operation().await else { \
     return UNAVAILABLE.into_response(); }; let reply = match entry {",
    "match auth::recognize(&state.app, request.headers()) {",
    "Err(refused) => { state.deny_anonymous(refused); refused.into_response() }",
    "self.recheck(caller, Entry::Board, &instance, &granted)?; let projects",
    "self.recheck(caller, Entry::Task, &instance, &granted)?; if reply",
    "self.recheck(caller, Entry::Sessions, &instance, &granted)?; Ok(Json(reply)",
    "for name in named { self.record(caller, &instance, &granted, name, Entry::Board)?; } \
     let mut response = Json(reply)",
    "self.record(caller, &instance, &granted, project, Entry::Task)?; Ok(Json(reply)",
];
/// `grant.rs` 里 `Grant` 这个名字出现的次数：再多一处，就可能是另建一份授权绕过那一判。
const GRANT_WORDS: usize = 7;
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
        // 整个 `AppState` 只有四处：字段、参数、放进字段、交给认证认人。
        && count(&all, "app: Arc<AppState>") == 2
        && count(&all, "app:") == 2
        && count(&all, "app,") == 2
        && count(&all, "Connect { app,") == 1
        && count(&all, "app.clone") == 0
        // 定义、实现、建一次：再多就是把字段拆开拿走。
        && count(&all, "Connect {") == 3
        // 子模块只有列名的两个：多一个，守卫就看不见它。
        && count(&all, "mod limits;") == 1
        && count(&all, "mod reply;") == 1
        && word(&all, "mod") == 2
        && word(&all, "super") == count(&all, "pub(super)")
        && followers(&all, "app.")
            .iter()
            .all(|field| STATE.contains(field))
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
        // 引擎两处都当场查完、不借成变量；两处都只许查询。
        && word(&tokens, "engine") == 2
        && count(&tokens, ".engine.lock().map_err(|_| Failure::Unavailable)?.board_list_all(") == 1
        && count(&tokens, ".engine.lock().map_err(|_| Failure::Unavailable)?.board_detail(") == 1
        // `BoardPlane` 的写方法写在别的文件里，从 `self` 上就调得到：用得到的逐个列名，也不许换个名字带走。
        && count(&tokens, HUMAN_USES) == 1
        && word(&tokens, "super") == 1 + count(&tokens, "pub(super)")
        && word(&tokens, "crate") == 2
        && count(&tokens, "use crate::editing::Problem;") == 1
        && count(&tokens, "crate::invalid_query(") == 1
        && word(&tokens, "mod") == 0
        && word(&tokens, "BoardPlane") == 4
        && count(&tokens, "impl BoardPlane {") == 1
        && count(&tokens, "Extension<Arc<BoardPlane>>") == 2
        && followers(&tokens, "self")
            .iter()
            .all(|next| [".", ",", ")", "{"].contains(next))
        && count(&tokens, "self {") == count(&tokens, "match self {")
        && count(&tokens, "(self)") == count(&tokens, "fn into_response(self)")
        && count(&tokens, "(self,") == 1
        && count(&tokens, "lineages.project(self, view)") == 1
        && borrowed_only_by_its_own_methods(&tokens)
        && followers(&tokens, "self.")
            .iter()
            .all(|name| HUMAN_SELF.contains(name) || *name == HUMAN_VIEW)
        && followers(&tokens, "plane")
            .iter()
            .all(|next| [".", ":", ")", "else"].contains(next))
        && count(&tokens, "plane else") == count(&tokens, "let Some(Extension(plane)) = plane else {")
        && count(&tokens, "(plane)") == count(&tokens, "Extension(plane)")
        && followers(&tokens, "plane.")
            .iter()
            .all(|name| HUMAN_PLANE.contains(name))
        && followers(&tokens, "Self::")
            .iter()
            .all(|name| name.chars().next().is_some_and(char::is_uppercase))
}

/// `&self` 后面不接着取字段、调方法的，只能是方法签名里那一个（`fn 名字(&self`）：不许整个借给别人。
fn borrowed_only_by_its_own_methods(tokens: &[String]) -> bool {
    (3..tokens.len() - 2)
        .filter(|&at| tokens[at] == "&" && tokens[at + 1] == "self" && tokens[at + 2] != ".")
        .all(|at| tokens[at - 1] == "(" && tokens[at - 3] == "fn")
}

/// `目录` 里的文件名，排好序。
fn files(directory: &str) -> Vec<String> {
    let mut names = fs::read_dir(workspace_root().join(directory))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// 调用的宏，逐处列出（`名字!(`、`名字![`、`名字!{`）。
fn macros(tokens: &[String]) -> Vec<&str> {
    tokens
        .windows(3)
        .filter(|window| {
            window[1] == "!"
                && ["(", "[", "{"].contains(&window[2].as_str())
                && window[0].chars().all(|c| c == '_' || c.is_alphanumeric())
                && !["if", "return", "while", "match", "in"].contains(&window[0].as_str())
        })
        .map(|window| window[0].as_str())
        .collect()
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
        // 子模块和宏都能把外面的东西带进来：产品代码里没有子模块，宏只有 `matches!`。
        && word(&tokens, "mod") == 0
        && macros(&tokens).iter().all(|name| *name == "matches")
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
        // 交得出授权的只有两处：兼容模式（没接策略平面）和过了上面那一判的。
        && count(&lex_product(grant), "Ok(Grant {") == 2
        && word(&lex_product(grant), "Grant") == GRANT_WORDS
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
    assert_eq!(files("crates/agent-server/src/connector"), AGENT_FILES);
    assert_eq!(
        files("crates/local-server/src/board/presence"),
        PRESENCE_FILES
    );
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
        // 多一个守卫看不见的子模块。
        (0, "mod reply;", "mod reply;\nmod escape;"),
        (
            1,
            "#[derive(Default)]\npub(super) struct Limits {",
            "mod escape;\n#[derive(Default)]\npub(super) struct Limits {",
        ),
        // 拆开自己的字段拿走整个 `AppState`。
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let Connect { app: whole, .. } = self; let _ = &whole.allow;\n        let Some(policy) = &self.app.policy else {",
        ),
        (
            0,
            "let Some(policy) = &self.app.policy else {",
            "let _ = elsewhere(&self.app, 1);\n        let Some(policy) = &self.app.policy else {",
        ),
        // 不先收完正文、不排名额、不取操作准入就裁。
        (
            0,
            "let Ok(_slot) = state.authority.acquire().await else {\n        return UNAVAILABLE.into_response();\n    };",
            "",
        ),
        (
            0,
            "let Some(_operation) = state.app.begin_operation().await else {\n        return UNAVAILABLE.into_response();\n    };",
            "let _ = state.app.begin_operation().await;",
        ),
        (
            0,
            "Entry::Sessions => match read(body).await {\n            Ok(body) => body,\n            Err(refusal) => return refusal.into_response(),\n        },\n    };",
            "Entry::Sessions => Bytes::new(),\n    };",
        ),
        // 没认出的拒绝不限频。
        (
            0,
            "            state.deny_anonymous(refused);\n",
            "            let _ = state.app.audit(every(refused));\n            state.deny_anonymous(refused);\n",
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
            "let mut lineages = Lineages::default();",
            "let engine = self.engine.lock();\n            engine.board_cancel(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            ".board_detail(task_id, &grant.hash)",
            ".board_claim(task_id, &grant.hash)",
        ),
        (
            ".board_list_all(&grant.hash)",
            ".board_reconcile(&grant.hash)",
        ),
        // 从 `self`、`plane` 上调 `BoardPlane` 写工作项的现成方法，或换个名字带走再调。
        (
            "let mut lineages = Lineages::default();",
            "let _ = self.execute(request);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let whole = self;\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(self, 1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(&self);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "elsewhere(self);\n        let mut lineages = Lineages::default();",
        ),
        (
            "        plane.connect(call)\n",
            "        elsewhere(plane)\n",
        ),
        (
            "let mut lineages = Lineages::default();",
            "BoardPlane::execute(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "let _ = Self::execute;\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "super::execute::run(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "let mut lineages = Lineages::default();",
            "crate::writer::run(1);\n        let mut lineages = Lineages::default();",
        ),
        (
            "        plane.connect(call)\n",
            "        plane.execute(call)\n",
        ),
        (
            "        plane.connect(call)\n",
            "        let whole = plane;\n        whole.execute(call)\n",
        ),
        (
            "execute::{Failure as BoardFailure, parse, value},",
            "execute::{Failure as BoardFailure, parse, run, value},",
        ),
        (
            "fn board_roster(set: &CapabilitySet)",
            "mod escape;\nfn board_roster(set: &CapabilitySet)",
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
        "mod escape;",
        "mod escape { pub fn now() {} }",
        "fn later() { let _ = env!(\"HOME\"); }",
        "fn later() { println![\"x\"]; }",
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
        // 另开一条交授权的路。
        (
            "        let whitelist = grant",
            "        if early { return Ok(Grant { whitelist: all }); }\n        let whitelist = grant",
        ),
        (
            "        let whitelist = grant",
            "        let early: Grant = all;\n        let whitelist = grant",
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
