//! 连接器入口（ADR-046）的边界：从结果上看不出来、或要卡在一瞬间才看得出来的几条靠这里守。
//!
//! - agent 那一半不碰供给副本，也不借 MCP 那一侧的代码：引入的东西、`AppState` 上的用法、文件逐个列名。
//! - agent 先收完正文、排上名额、取到操作准入，才碰身份库和策略库。
//! - 人侧账本只记两样：新会话第一次出现、限频的拒绝；更新和过期不进账本，先登记后落下。
//! - 人侧只读引擎的两个查询，不改任何工作项；`BoardPlane` 上的用法逐个列名，不许整个交给别人。
//! - 状态的纯逻辑不碰文件、网络和时钟：引入的东西逐个列名。
//! - 连接器类型进不了 MCP 和 `/read`：在每次复核都要过的那一处判，只此一处。
//! - 排队、等锁期间撤了权就一项不给：交付前的再复核不能少。
//!
//! 按记号比（`rust_lexer`）：注释掉的、写进字符串的都不算数；挪位置、夹条件也算没有。挡的是改漏和顺手越界；
//! 存心绕（派生宏、属性字符串里的路径、改名套改名）按记号看不全，最后靠复查。每种改法的例子在 `connector_entry_ways`。
//!
//! 判红时先想是不是真要多开这一样：要，就在这里的名单加上并说明为什么不越界；不要，就改回去。
use crate::support::{
    rust_lexer::{lex, lex_product},
    workspace_root,
};
use std::{fs, path::Path};

pub(crate) const AGENT: [&str; 3] = [
    "crates/agent-server/src/connector.rs",
    "crates/agent-server/src/connector/limits.rs",
    "crates/agent-server/src/connector/reply.rs",
];
pub(crate) const HUMAN: &str = "crates/local-server/src/board/connector.rs";
pub(crate) const PRESENCE: &str = "crates/local-server/src/board/presence.rs";
pub(crate) const GRANT: &str = "crates/agent-server/src/server/grant.rs";

/// 入口的子目录里只有这两个文件：守卫只看得到列名的。
const AGENT_FILES: [&str; 2] = ["limits.rs", "reply.rs"];
/// 状态那一侧的子目录里只有用例。
const PRESENCE_FILES: [&str; 1] = ["tests.rs"];

/// 一个文件能从外面拿的东西。
struct Reach {
    /// 全部 `use` 段，按出现的次序原样。
    uses: &'static [&'static str],
    /// `use` 之外写出来的小写路径的开头：一段的整段放行，两段的只放行这一个。大写开头的不用列：要么在上面引入过，
    /// 要么是自己的、标准库默认给的。
    paths: &'static [&'static str],
    macros: &'static [&'static str],
    /// 写死次数的词。
    words: &'static [(&'static str, usize)],
}

const AGENT_REACH: [Reach; 3] = [
    Reach {
        uses: &[
            "use crate::{ auth::{self, AgentType, CredentialDigest, Unrecognized}, \
             board_client::ConnectorError, http_text::strict_query, shutdown::SERVER_STOPPING, \
             state::{AppState, now}, };",
            "use axum::{ Json, Router, body::{Body, Bytes, to_bytes}, extract::State, \
             http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header, request::Parts}, \
             middleware::{self, Next}, response::{IntoResponse, Response}, };",
            "use limits::Limits;",
            "use reply::{Refusal, *};",
            "use serde_json::{Value, json};",
            "use sha2::{Digest as _, Sha256};",
            "use std::{ collections::BTreeMap, sync::{Arc, Mutex}, time::{Duration, Instant}, };",
            "use studiolo_policy::CapabilitySet;",
            "use studiolo_supply::audit::{self, Actor};",
            "use tokio::sync::Semaphore;",
        ],
        paths: &[
            "audit::action_denied",
            "audit::content_read",
            "audit::instance_registered",
            "auth::recognize",
            "collect",
            "get",
            "header",
            "middleware",
            "serde_json::from_slice",
            "serde_json::to_vec",
            "tokio::time",
            "ulid::Ulid",
        ],
        macros: &["format", "json"],
        words: &[("mod", 2), ("AppState", 2)],
    },
    Reach {
        uses: &["use std::{ collections::{BTreeMap, VecDeque}, time::{Duration, Instant}, };"],
        paths: &[],
        macros: &[],
        words: &[("mod", 0), ("AppState", 0)],
    },
    Reach {
        uses: &[
            "use axum::{ Json, body::Body, http::{HeaderName, HeaderValue, Request, StatusCode, header}, \
             middleware::Next, response::{IntoResponse, Response}, };",
        ],
        paths: &["header", "serde_json::json"],
        macros: &["json"],
        words: &[("mod", 0), ("AppState", 0)],
    },
];
const HUMAN_REACH: Reach = Reach {
    uses: &[
        "use super::{ BoardPlane, auth::{Grant, IDENTITY_UNAVAILABLE, Refusal}, \
         execute::{Failure as BoardFailure, parse, value}, http::{busy, unavailable, unconfigured}, \
         overview::{Item, Lineages, terminal}, \
         presence::{EXPIRES, Fresh, Origin, Presence, Reason, Refused, Snapshot}, \
         requests::{self, Task}, };",
        "use crate::editing::Problem;",
        "use axum::{ Extension, Json, extract::{Query, rejection::JsonRejection, rejection::QueryRejection}, \
         http::StatusCode, response::{IntoResponse, Response}, };",
        "use serde::{Deserialize, Serialize};",
        "use serde_json::{Value, json};",
        "use std::{ collections::BTreeMap, sync::{Arc, Mutex, MutexGuard}, time::{Duration, Instant}, };",
        "use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};",
        "use studiolo_policy::CapabilitySet;",
    ],
    paths: &[
        "collect",
        "crate::invalid_query",
        "jiff::Timestamp",
        "requests::Request",
        "serde_json::to_value",
        "serde_json::to_vec",
        "tokio::task",
        "ulid::Ulid",
    ],
    macros: &["format", "json"],
    words: &[
        ("mod", 0),
        // 参数两处、`impl` 一处；再多就可能拿整个来用。
        ("BoardPlane", 3),
        ("append", 1),
        ("engine", 2),
    ],
};
const PRESENCE_REACH: Reach = Reach {
    uses: &[
        "use serde::{Deserialize, Serialize};",
        "use std::{ collections::{BTreeMap, BTreeSet, VecDeque}, time::Duration, };",
        "use studiolo_core::VaultPath;",
        "use studiolo_policy::{connector_project, invisible};",
    ],
    paths: &[
        "collect",
        "serde_json::Value",
        "serde_json::from_value",
        "sum",
        "u64",
    ],
    macros: &["matches"],
    words: &[("mod", 0)],
};

/// agent 那一半在 `AppState` 上的全部用法；供给副本、白名单、会话都不在里面，身份库只许这三个方法。
const APP: [&str; 8] = [
    "app.audit(event)",
    "app.begin_operation().await",
    "app.board.as_ref().ok_or(UNAVAILABLE)?;",
    "app.lifecycle.is_stopping()",
    "app.policy else {",
    "app.store.credential_issued_at(",
    "app.store.register_instance(",
    "app.store.touch_instance(",
];
/// 人侧那一半在 `BoardPlane`（和状态口自己）上的全部用法：库句柄只到列名的方法，其余是本文件里的方法和授权。
/// 写工作项的 `commit`、`project` 这类现成方法不在里面。
const HUMAN_SELF: [&str; 22] = [
    "self.audit.append(AuditEvent::new(",
    "self.audit.try_enter_operation()",
    "self.confirm(",
    "self.connector_board(",
    "self.connector_grant(",
    "self.connector_sessions(",
    "self.connector_task(",
    "self.engine.lock()",
    "self.granted(",
    "self.identities.types()",
    "self.issued_at(",
    "self.note_denial(",
    "self.presence.lock()",
    "self.presence.since,",
    "self.presence.started.elapsed()",
    "self.presence_view(",
    "self.record(json!({",
    "self.record_fresh(",
    "self.refresh_presence(",
    "self.rostered(",
    "self.state.lock()",
    "self.storage.check()",
];
/// 入口处理函数手里的 `plane` 的全部用法。
const HUMAN_PLANE: [&str; 4] = [
    "plane.connect(call)",
    "plane.connectors.clone()",
    "plane.permits.clone()",
    "plane.presence_view(None)",
];
/// 人侧那一半要是活代码、各恰好一处的几段。
const HUMAN_ANCHORS: [&str; 12] = [
    // `plane` 从哪来：换个名字接，下面按名字查的那条就看不见了。
    "pub(super) async fn command( Extension(plane): Extension<Arc<BoardPlane>>, call:",
    "pub(super) async fn presence( plane: Option<Extension<Arc<BoardPlane>>>, query:",
    "let Some(Extension(plane)) = plane else { return unconfigured(); };",
    // 整个交出去的只有这一处：查实例链的只读帮手。
    "lineages.project(self, view)",
    "let (status, code, error) = match self {",
    "fn into_response(self) -> Response {",
    // 引擎两处都当场查完、不借成变量；两处都只许查询。
    ".engine.lock().map_err(|_| Failure::Unavailable)?.board_list_all(&grant.hash)",
    ".engine.lock().map_err(|_| Failure::Unavailable)?.board_detail(task_id, &grant.hash)",
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
/// agent 那一半要是活代码、各恰好一处的几段：正文先收完再占名额，排上名额、取到操作准入才裁；交付前再复核、先审计后给；没认出的拒绝限频。
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
/// 整个 `AppState` 只有三处：参数、放进字段、交给认证认人。
const APP_WHOLE: [(&str, usize); 4] = [
    ("app: Arc<AppState>", 2),
    ("app:", 2),
    ("Connect { app,", 1),
    ("app,", 2),
];
/// `grant.rs` 里 `Grant` 这个名字出现的次数：再多一处，就可能是另建一份授权绕过那一判。
const GRANT_WORDS: usize = 7;
/// 交得出授权的只有两处：兼容模式（没接策略平面），和过了连接器那一判的。
const GRANT_COMPATIBLE: &str = "let Some(policy) = &self.state.policy else { return Ok(Grant {";
const GRANT_ANCHOR: &str = "let grant = authorization.grant.ok_or_else(|| \
     McpError::internal_error(\"\", None))?; if grant.set.connector_only() { self.deny( \
     Actor::instance(&instance.agent_type, &instance.id), action, \"\", ); \
     return Err(McpError::invalid_request( format!( \"\" ), None, )); } let whitelist =";

pub(crate) fn source(path: &str) -> String {
    fs::read_to_string(workspace_root().join(path)).unwrap()
}

fn count(tokens: &[String], fragment: &str) -> usize {
    let fragment = lex(fragment);
    tokens
        .windows(fragment.len())
        .filter(|window| *window == fragment)
        .count()
}

fn word(tokens: &[String], name: &str) -> usize {
    tokens.iter().filter(|token| *token == name).count()
}

/// `prefix` 的每一处都得是名单里某一种写法的开头；不是的逐处写出来。
fn strays(tokens: &[String], prefix: &str, allowed: &[&str]) -> Vec<String> {
    let prefix = lex(prefix);
    let allowed = allowed.iter().map(|one| lex(one)).collect::<Vec<_>>();
    (0..tokens.len())
        .map(|at| &tokens[at..])
        .filter(|rest| rest.starts_with(&prefix))
        .filter(|rest| !allowed.iter().any(|one| rest.starts_with(one)))
        .map(|rest| rest.iter().take(8).cloned().collect::<Vec<_>>().join(" "))
        .collect()
}

/// 恰好 `times` 处；不是就写出来。
fn exactly(tokens: &[String], fragment: &str, times: usize) -> Option<String> {
    let found = count(tokens, fragment);
    (found != times).then(|| format!("应有 {times} 处、实有 {found} 处：{fragment}"))
}

/// 拆成 `use` 段（原样）和其余记号。
fn split_uses(tokens: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut uses, mut rest, mut at) = (Vec::new(), Vec::new(), 0);
    while at < tokens.len() {
        if tokens[at] == "use" {
            let length = tokens[at..].iter().position(|token| token == ";");
            let end = length.map_or(tokens.len(), |length| at + length + 1);
            uses.push(tokens[at..end].join(" "));
            at = end;
        } else {
            rest.push(tokens[at].clone());
            at += 1;
        }
    }
    (uses, rest)
}

fn is_word(token: &str) -> bool {
    token.chars().all(|c| c == '_' || c.is_alphanumeric())
}

/// 引入、直接写出来的路径、宏都在名单里；返回 `use` 段以外的记号和越界的地方。
fn reach(source: &str, reach: &Reach) -> (Vec<String>, Vec<String>) {
    let (uses, rest) = split_uses(&lex_product(source));
    let mut found = Vec::new();
    let expected = reach.uses.iter().map(|line| lex(line).join(" "));
    if !uses.iter().eq(&expected.collect::<Vec<_>>()) {
        found.push(format!("引入的不是列名的那些：{uses:?}"));
    }
    for at in 1..rest.len().saturating_sub(1) {
        let (before, next) = (rest[at - 1].as_str(), rest[at + 1].as_str());
        if rest[at] == "::" && !before.starts_with(char::is_uppercase) {
            // 前面不是名字的是从类型（`<T>::`）或根（`::std`）上取的，一律不许。
            let inside = is_word(before) && at >= 2 && rest[at - 2] == "::";
            let pair = format!("{before}::{next}");
            if !inside && !reach.paths.contains(&before) && !reach.paths.contains(&pair.as_str()) {
                found.push(format!("名单外的路径：{pair}"));
            }
        }
        // 比较、取反前面是关键字；前面是名字的就是宏。
        if rest[at] == "!"
            && is_word(before)
            && !["if", "return", "while", "match", "in"].contains(&before)
            && !reach.macros.contains(&before)
        {
            found.push(format!("名单外的宏：{before}!"));
        }
    }
    for (name, times) in reach.words.iter().chain(&[("extern", 0), ("unsafe", 0)]) {
        found.extend(exactly(&rest, name, *times));
    }
    // 模块文件不许另指。
    found.extend(exactly(&rest, "path =", 0));
    (rest, found)
}

/// agent 入口的三个文件合起来看。
pub(crate) fn agent_problems(sources: &[String; 3]) -> Vec<String> {
    let (mut all, mut found) = (Vec::new(), Vec::new());
    for (source, limits) in sources.iter().zip(&AGENT_REACH) {
        let (tokens, problems) = reach(source, limits);
        all.extend(tokens);
        found.extend(problems);
    }
    found.extend(strays(&all, "app.", &APP));
    for (fragment, times) in APP_WHOLE {
        found.extend(exactly(&all, fragment, times));
    }
    // 除此以外只许 `.app.…`：换个名字拿走整个 `AppState` 就绕过了上面那条。
    let whole = word(&all, "app") - count(&all, "app.") - count(&all, "app:") - count(&all, "app,");
    if whole != 0 {
        found.push(format!("另有 {whole} 处拿着整个 AppState"));
    }
    // 权威服务的客户端只许走连接器那一个方法。
    found.extend(exactly(&all, "client", 2));
    found.extend(exactly(&all, "client.connect(&json!({", 1));
    // 策略库只许问授权。
    found.extend(exactly(
        &all,
        "let Some(policy) = &self.app.policy else {",
        1,
    ));
    found.extend(exactly(&all, "policy", 3));
    found.extend(exactly(&all, "policy.authorization(", 1));
    found.extend(
        AGENT_ANCHORS
            .iter()
            .filter_map(|anchor| exactly(&all, anchor, 1)),
    );
    found
}

pub(crate) fn human_problems(source: &str) -> Vec<String> {
    let (tokens, mut found) = reach(source, &HUMAN_REACH);
    found.extend(
        HUMAN_ANCHORS
            .iter()
            .filter_map(|anchor| exactly(&tokens, anchor, 1)),
    );
    for (fragment, times) in [
        ("self.record(", 2),
        ("self.record_fresh(", 1),
        ("presence.commit(", 1),
    ] {
        found.extend(exactly(&tokens, fragment, times));
    }
    // 写成类型的只有两个返回值、建状态口的一处；多一个就可能是拿整个 `BoardPlane` 的参数。
    let named = word(&tokens, "Self") - count(&tokens, "Self::");
    if named != 3 {
        found.push(format!("`Self` 当类型用了 {named} 处，应是 3 处"));
    }
    // `Self::` 后面只许枚举的取值：小写的是 `BoardPlane` 的方法。
    found.extend(
        tokens
            .windows(3)
            .filter(|window| window[0] == "Self" && window[1] == "::")
            .filter(|window| !window[2].starts_with(char::is_uppercase))
            .map(|window| window.concat()),
    );
    // `BoardPlane` 的写方法写在别的文件里，从 `self`、`plane` 上就调得到：用法逐个列名，也不许换个名字带走。
    found.extend(strays(&tokens, "self.", &HUMAN_SELF));
    found.extend(strays(&tokens, "plane.", &HUMAN_PLANE));
    let whole = whole_self(&tokens);
    if !whole.is_empty() {
        found.push(format!("整个交出去的 self：{whole:?}"));
    }
    // 三处绑定之外只许 `plane.…`。
    let named = word(&tokens, "plane") - count(&tokens, "plane.");
    if named != 4 {
        found.push(format!("`plane` 除了取用还出现 {named} 处，应是 4 处"));
    }
    found
}

/// 不接着取字段、调方法的 `self`：除了方法签名里那一个和列明的三处，别的都算整个交出去。
fn whole_self(tokens: &[String]) -> Vec<String> {
    let whole = (4..tokens.len() - 1)
        .filter(|&at| tokens[at] == "self" && tokens[at + 1] != ".")
        .filter(|&at| !(tokens[at - 1] == "&" && tokens[at - 2] == "(" && tokens[at - 4] == "fn"))
        .map(|at| tokens[at - 4..=at + 1].join(" "))
        .collect::<Vec<_>>();
    // 列明的三处（上面各钉了恰好一处）都在里面；多一处就是新的。
    if whole.len() == 3 { Vec::new() } else { whole }
}

pub(crate) fn presence_problems(source: &str) -> Vec<String> {
    reach(source, &PRESENCE_REACH).1
}

pub(crate) fn grant_problems(grant: &str, others: &[String]) -> Vec<String> {
    let tokens = lex_product(grant);
    let mut found = [
        (GRANT_ANCHOR, 1),
        (GRANT_COMPATIBLE, 1),
        ("Ok(Grant {", 2),
        ("Grant", GRANT_WORDS),
        // 换个名字建的也算。
        ("Self", 0),
        ("connector_only", 1),
    ]
    .iter()
    .filter_map(|(fragment, times)| exactly(&tokens, fragment, *times))
    .collect::<Vec<_>>();
    // 别处另判一遍会走偏；别处另建一份授权就绕过了那一判。
    for other in others {
        let tokens = lex_product(other);
        found.extend(exactly(&tokens, "connector_only", 0));
        found.extend(exactly(&tokens, "Grant {", 0));
    }
    found
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

/// `crates/agent-server/src` 里 `grant.rs` 以外的全部源码。
pub(crate) fn other_agent_sources() -> Vec<String> {
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
        let none = Vec::<String>::new();
        let agent = AGENT.map(|path| reformat(source(path)));
        assert_eq!(agent_problems(&agent), none, "agent 入口");
        assert_eq!(human_problems(&reformat(source(HUMAN))), none, "{HUMAN}");
        let presence = reformat(source(PRESENCE));
        assert_eq!(presence_problems(&presence), none, "{PRESENCE}");
        let grant = reformat(source(GRANT));
        assert_eq!(
            grant_problems(&grant, &other_agent_sources()),
            none,
            "{GRANT}"
        );
    }
}
