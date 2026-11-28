//! `read_note` 按行段、按标题、只看大纲的真实 loopback 契约（T-255）。
//! 覆盖：不带新参数逐字节不变、行段与越界、标题节的边界与同名、大纲与读模型一致、
//! 审计与整篇读同一条记法、白名单拒绝不变、三选一、托管副本的新鲜度行。

use std::{
    fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};

use serde_json::{Value, json};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_supply::{AgentStore, ReadQuery};
use tempfile::TempDir;

#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;
#[allow(dead_code)]
#[path = "replica_http/support.rs"]
mod support;

include!("mcp_contract_support/fixture.rs");

const INVALID_PARAMS: i64 = -32602;
const RESOURCE_NOT_FOUND: i64 = -32002;

const SMALL: &str = "项目/Alpha/小.md";
/// 12 行；二级节里有三级子节，最后又回到一级。
const SMALL_BODY: &str = "# 一\n\n一的正文\n## 二\n二的正文\n### 三\n三的正文\n## 二\n另一个二\n# 四\n四的正文\n末行没有换行";

const TRICKY: &str = "项目/Alpha/怪.md";
/// 标题识别的各种边角；与读模型的大纲逐条对照。
const TRICKY_BODY: &str = concat!(
    "\u{feff}---\r\n",
    "title: 怪\r\n",
    "# frontmatter 里不是标题\r\n",
    "---\r\n",
    "# 一 \r\n",
    "```js\n# 围栏里不是\n``` 行尾有字不算闭合\n# 还在围栏里\n```\n",
    "~~~~\n## 不是\n~~~\n`````\n## 还不是\n~~~~~  \n",
    "    ## 缩进四格也算\n",
    "\t# 缩进一个制表符也算\n",
    "    ```\n# 四格缩进的反引号不开围栏\n",
    "``\n## 两个反引号不开围栏\n",
    "#没有空格不是\n",
    "####### 七个井号不是\n",
    "##\n",
    "##   \n",
    "##\t制表符隔开\n",
    "## 尾井号 ##\n",
    "### C#\n",
    "#### 只剩井号 # #\n",
    "```a`b 的 info 里有反引号，不开围栏\n# 所以这是标题\n",
    "- # 列表里不是\n",
    "## 尾巴\r\n",
    "```\n# 没闭合的围栏里不是\n",
);

const VARIANTS: [(&str, &str); 6] = [
    ("项目/Alpha/空.md", ""),
    ("项目/Alpha/回车换行.md", "# 甲\r\n乙\r\n丙"),
    ("项目/Alpha/没有换行.md", "只有一行"),
    ("项目/Alpha/尾换行.md", "一\n二\n\n"),
    (
        "项目/Alpha/没闭合的头.md",
        "---\na: b\n# 不是标题\n## 也不是\n",
    ),
    (
        "项目/Alpha/点点点收尾.md",
        "---\na: b\n...\n正文\n# 是标题\n",
    ),
];

async fn notes() -> (Fixture, String) {
    let f = fixture_seeded(
        serve::router,
        |vault| {
            write(&vault.join(SMALL), SMALL_BODY);
            write(&vault.join(TRICKY), TRICKY_BODY);
            for (path, body) in VARIANTS {
                write(&vault.join(path), body);
            }
        },
        &[],
    )
    .await;
    let (session, _) = f.initialize(&f.token).await;
    (f, session)
}

/// 调 `read_note`；`part` 里的键并进参数。
async fn read(f: &Fixture, session: &str, path: &str, part: Value) -> Value {
    let mut arguments = json!({"path": path});
    for (key, value) in part.as_object().unwrap() {
        arguments[key] = value.clone();
    }
    f.call(&f.token, session, 9, "read_note", arguments).await
}

async fn text(f: &Fixture, session: &str, path: &str, part: Value) -> String {
    let response = read(f, session, path, part.clone()).await;
    assert!(!is_error(&response), "{path} {part}: {response}");
    tool_text(&response)
}

/// 去掉首行 `revision …`，返回（首行, 其余）。
fn after_revision(text: &str) -> (&str, &str) {
    let (first, rest) = text.split_once('\n').unwrap();
    assert!(first.starts_with("revision "), "{first}");
    (first, rest)
}

/// 再去掉切段的头行，返回（头行, 正文）。
fn after_head(text: &str) -> (&str, &str) {
    after_revision(text).1.split_once('\n').unwrap()
}

fn refusal(response: &Value) -> (i64, String, Value) {
    let error = &response["error"];
    (
        error["code"].as_i64().unwrap_or_default(),
        error["message"].as_str().unwrap_or_default().to_owned(),
        error["data"].clone(),
    )
}

fn recorded(f: &Fixture, action: &str) -> Vec<Value> {
    f.ledger()
        .query(&AuditFilter::default().with_action(action))
        .unwrap()
        .iter()
        .map(|record| record.event().payload().as_value().clone())
        .collect()
}

#[tokio::test]
async fn without_slice_arguments_the_note_comes_back_byte_for_byte() {
    let (f, session) = notes().await;
    let mut bodies = vec![(SMALL, SMALL_BODY), (TRICKY, TRICKY_BODY)];
    bodies.extend(VARIANTS);
    for (path, body) in bodies {
        for part in [
            json!({}),
            json!({"outline": false}),
            json!({"lines": null, "heading": null, "outline": null}),
        ] {
            let whole = text(&f, &session, path, part.clone()).await;
            let (first, rest) = after_revision(&whole);
            assert_eq!(rest, body, "{path} {part}");
            let hex = first.strip_prefix("revision ").unwrap();
            assert_eq!(hex.len(), 64, "{first}");
        }
    }
}

#[tokio::test]
async fn lines_returns_only_that_range_and_says_how_many_lines_there_are() {
    let (f, session) = notes().await;
    let whole = text(&f, &session, SMALL, json!({})).await;
    for (range, head, body) in [
        ("1-1", "# 行 1-1 · 共 12 行", "# 一\n"),
        ("2-4", "# 行 2-4 · 共 12 行", "\n一的正文\n## 二\n"),
        ("12-12", "# 行 12-12 · 共 12 行", "末行没有换行"),
        ("1-12", "# 行 1-12 · 共 12 行", SMALL_BODY),
    ] {
        let part = text(&f, &session, SMALL, json!({"lines": range})).await;
        assert_eq!(after_revision(&part).0, after_revision(&whole).0);
        assert_eq!(after_head(&part), (head, body), "{range}");
    }
    let part = text(
        &f,
        &session,
        "项目/Alpha/回车换行.md",
        json!({"lines": "2-3"}),
    )
    .await;
    assert_eq!(after_head(&part), ("# 行 2-3 · 共 3 行", "乙\r\n丙"));
    let part = text(
        &f,
        &session,
        "项目/Alpha/尾换行.md",
        json!({"lines": "3-3"}),
    )
    .await;
    assert_eq!(after_head(&part), ("# 行 3-3 · 共 3 行", "\n"));

    // 行号和 search 报的是同一套：搜到哪一行，就能只读那一行。
    let found = f
        .call(
            &f.token,
            &session,
            5,
            "search",
            json!({"project": "项目/Alpha", "text": "四的正文"}),
        )
        .await;
    let found = tool_text(&found);
    let line = found
        .lines()
        .find_map(|hit| hit.strip_prefix(&format!("{SMALL}:")))
        .and_then(|rest| rest.split_once('\t'))
        .unwrap()
        .0
        .to_owned();
    let part = text(
        &f,
        &session,
        SMALL,
        json!({"lines": format!("{line}-{line}")}),
    )
    .await;
    assert_eq!(after_head(&part).1, "四的正文\n");
}

#[tokio::test]
async fn a_range_that_is_malformed_backwards_or_out_of_bounds_is_refused_without_content() {
    let (f, session) = notes().await;
    for bad in [
        "",
        "1",
        "1-",
        "-1",
        "-",
        "0-0",
        "0-3",
        "3-2",
        "1-3-5",
        " 1-2",
        "1 - 2",
        "+1-2",
        "1-2 ",
        "a-b",
        "１-２",
        "1-99999999999999999999",
        "1..2",
        "01-x",
    ] {
        let response = read(&f, &session, SMALL, json!({"lines": bad})).await;
        let (code, message, data) = refusal(&response);
        assert_eq!(code, INVALID_PARAMS, "{bad:?}: {response}");
        assert!(message.starts_with("lines 只能写成 a-b"), "{message}");
        assert!(data.is_null());
    }
    // 写法不对是在取笔记之前拒的：不记读取，也不记越权。
    assert!(recorded(&f, "content.read").is_empty());
    assert!(recorded(&f, "agent.action_denied").is_empty());

    for (path, range, total) in [
        (SMALL, "12-13", 12),
        (SMALL, "13-13", 12),
        (SMALL, "1-4294967295", 12),
        ("项目/Alpha/空.md", "1-1", 0),
    ] {
        let response = read(&f, &session, path, json!({"lines": range})).await;
        let (code, message, data) = refusal(&response);
        assert_eq!(code, INVALID_PARAMS, "{range}: {response}");
        assert_eq!(message, format!("行段越界：这篇共 {total} 行"));
        assert!(data.is_null());
        assert!(!response.to_string().contains("末行没有换行"));
    }
}

#[tokio::test]
async fn heading_returns_the_section_up_to_the_next_heading_of_the_same_or_a_higher_level() {
    let (f, session) = notes().await;
    for (query, head, body) in [
        // 子节算在里面；到下一个一级标题为止。
        (
            "一",
            "# 行 1-9 · 共 12 行",
            "# 一\n\n一的正文\n## 二\n二的正文\n### 三\n三的正文\n## 二\n另一个二\n",
        ),
        // 同名取第一处，头行说清还有别的。
        (
            "二",
            "# 行 4-7 · 共 12 行 · 同名标题 2 处，这是第 1 处，其余用 outline 查行号",
            "## 二\n二的正文\n### 三\n三的正文\n",
        ),
        // 到下一个更高级的标题就停，不会串进别的章节。
        ("三", "# 行 6-7 · 共 12 行", "### 三\n三的正文\n"),
        // 最后一节读到文末。
        (
            "四",
            "# 行 10-12 · 共 12 行",
            "# 四\n四的正文\n末行没有换行",
        ),
        // 整行贴过来也认，级别一起核。
        ("### 三", "# 行 6-7 · 共 12 行", "### 三\n三的正文\n"),
        (
            "  # 四  ",
            "# 行 10-12 · 共 12 行",
            "# 四\n四的正文\n末行没有换行",
        ),
    ] {
        let part = text(&f, &session, SMALL, json!({"heading": query})).await;
        assert_eq!(after_head(&part), (head, body), "{query}");
    }
    // 大纲怎么写，heading 就能怎么写：结尾的井号带不带都认。
    for query in ["C#", "C", "### C#", "### C"] {
        let part = text(&f, &session, TRICKY, json!({"heading": query})).await;
        assert!(after_head(&part).1.starts_with("### C#\n"), "{query}");
    }
}

#[tokio::test]
async fn a_heading_that_is_not_there_gets_one_fixed_sentence_and_nothing_else() {
    let (f, session) = notes().await;
    for (path, query) in [
        (SMALL, "没有这一节 canary"),
        (SMALL, "一的正文"),
        (SMALL, "## 一"),
        (SMALL, "### 二"),
        (TRICKY, "围栏里不是"),
        (TRICKY, "frontmatter 里不是标题"),
        (TRICKY, "没闭合的围栏里不是"),
        (TRICKY, "###"),
        ("项目/Alpha/没闭合的头.md", "不是标题"),
        ("项目/Alpha/空.md", "一"),
    ] {
        let response = read(&f, &session, path, json!({"heading": query})).await;
        assert_eq!(
            refusal(&response),
            (RESOURCE_NOT_FOUND, "没有这个标题".to_owned(), Value::Null),
            "{path} {query}: {response}"
        );
    }
    // 只有井号：当普通文字找，找不到；不当成「任意标题」。
    for marks in ["##", "## "] {
        let response = read(&f, &session, SMALL, json!({"heading": marks})).await;
        assert_eq!(refusal(&response).1, "没有这个标题", "{marks:?}");
    }
    for blank in ["", "  "] {
        let response = read(&f, &session, SMALL, json!({"heading": blank})).await;
        let (code, message, _) = refusal(&response);
        assert_eq!(
            (code, message.as_str()),
            (INVALID_PARAMS, "heading 不能为空")
        );
    }
}

#[tokio::test]
async fn outline_lists_every_heading_with_the_range_of_its_section() {
    let (f, session) = notes().await;
    let part = text(&f, &session, SMALL, json!({"outline": true})).await;
    assert_eq!(
        after_head(&part),
        (
            "# 大纲 · 5 节 · 共 12 行",
            "1-9\t# 一\n4-7\t## 二\n6-7\t### 三\n8-9\t## 二\n10-12\t# 四\n"
        )
    );
    for path in ["项目/Alpha/空.md", "项目/Alpha/没有换行.md"] {
        let part = text(&f, &session, path, json!({"outline": true})).await;
        assert!(after_head(&part).0.starts_with("# 大纲 · 0 节 · 共 "));
        assert_eq!(after_head(&part).1, "");
    }
}

#[tokio::test]
async fn the_outline_agrees_with_the_read_model_and_its_ranges_read_the_same_bytes() {
    let (f, session) = notes().await;
    let generation = f.state.current();
    let mut paths = vec![SMALL, TRICKY, "项目/Alpha/00-总览/INDEX.md"];
    paths.extend(VARIANTS.map(|(path, _)| path));
    for path in paths {
        let expected = generation
            .model
            .navigation(&VaultPath::parse(path).unwrap())
            .unwrap()
            .outline
            .iter()
            .map(|entry| (entry.source.line, entry.level, entry.title.clone()))
            .collect::<Vec<_>>();
        let part = text(&f, &session, path, json!({"outline": true})).await;
        let mut seen = Vec::new();
        for row in after_head(&part).1.lines() {
            let (range, heading) = row.split_once('\t').unwrap();
            let start = range.split_once('-').unwrap().0.parse::<u32>().unwrap();
            let (marks, title) = heading.split_once(' ').unwrap();
            let level = u8::try_from(marks.len()).unwrap();
            let first = !seen
                .iter()
                .any(|(_, known, name)| (*known, name) == (level, &title.to_owned()));
            seen.push((start, level, title.to_owned()));
            // 大纲给的行段直接交给 lines，和按标题读到的逐字节一样（同名的只有第一处按标题读得到）。
            let by_lines = text(&f, &session, path, json!({"lines": range})).await;
            if first {
                let by_heading = text(&f, &session, path, json!({"heading": heading})).await;
                assert_eq!(after_head(&by_lines).1, after_head(&by_heading).1, "{row}");
            }
        }
        assert_eq!(seen, expected, "{path}");
    }
    // 这一篇确实覆盖到了边角，不是两边都空着对上的。
    let part = text(&f, &session, TRICKY, json!({"outline": true})).await;
    assert!(
        after_head(&part).0.starts_with("# 大纲 · 11 节 · "),
        "{part}"
    );
}

#[tokio::test]
async fn every_way_of_reading_is_audited_exactly_like_a_whole_read() {
    let (f, session) = notes().await;
    let whole = text(&f, &session, SMALL, json!({})).await;
    let first = recorded(&f, "content.read");
    assert_eq!(first.len(), 1);
    let revision = after_revision(&whole)
        .0
        .strip_prefix("revision ")
        .unwrap()
        .to_owned();
    assert_eq!(first[0]["revision"], json!(revision));
    for (part, refused) in [
        (json!({"lines": "1-2"}), false),
        (json!({"heading": "三"}), false),
        (json!({"outline": true}), false),
        // 说「共几行」「没有这个标题」也是在说正文的事，一样先记再答。
        (json!({"lines": "90-99"}), true),
        (json!({"heading": "没有"}), true),
    ] {
        let response = read(&f, &session, SMALL, part.clone()).await;
        assert_eq!(is_error(&response), refused, "{part}: {response}");
    }
    let reads = recorded(&f, "content.read");
    assert_eq!(reads.len(), 6);
    assert!(reads.iter().all(|read| *read == first[0]), "{reads:?}");
    assert!(recorded(&f, "agent.action_denied").is_empty());
    let indexed = f
        .store()
        .query_reads(&ReadQuery {
            path_prefix: Some(SMALL.into()),
            limit: 20,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(indexed.len(), 6);
    assert!(
        indexed
            .iter()
            .all(|read| read.tool == "read_note" && read.revision == revision)
    );
}

#[tokio::test]
async fn paths_outside_the_allowlist_are_refused_in_the_same_words_whatever_the_part() {
    let (f, session) = notes().await;
    for (path, reason) in [
        ("School/secret.md", "outside_allowlist"),
        ("项目/Beta/00-总览/INDEX.md", "outside_allowlist"),
        ("../School/secret.md", "unsafe_path"),
        ("项目/Alpha/missing.md", "not_found"),
    ] {
        let plain = refusal(&read(&f, &session, path, json!({})).await);
        for part in [
            json!({"lines": "1-2"}),
            json!({"heading": "私人成绩 秘密内容"}),
            json!({"outline": true}),
        ] {
            let response = read(&f, &session, path, part.clone()).await;
            assert_eq!(refusal(&response), plain, "{path} {part}");
        }
        let denied = recorded(&f, "agent.action_denied");
        assert!(
            denied.iter().all(|event| event["action"] == "read_note"),
            "{denied:?}"
        );
        assert_eq!(denied.last().unwrap()["reason"], reason);
    }
    // 参数写得不对也盖不住越界：先判路径，再看参数。
    for part in [
        json!({"lines": "x"}),
        json!({"heading": ""}),
        json!({"lines": "1-2", "outline": true}),
    ] {
        let response = read(&f, &session, "School/secret.md", part.clone()).await;
        assert_eq!(
            refusal(&response).1,
            "路径不在供给白名单内",
            "{part}: {response}"
        );
    }
    assert!(recorded(&f, "content.read").is_empty());
}

#[tokio::test]
async fn only_one_of_the_three_ways_can_be_asked_for() {
    let (f, session) = notes().await;
    for part in [
        json!({"lines": "1-2", "heading": "一"}),
        json!({"lines": "1-2", "outline": true}),
        json!({"heading": "一", "outline": true}),
        json!({"lines": "1-2", "heading": "一", "outline": true}),
    ] {
        let response = read(&f, &session, SMALL, part.clone()).await;
        let (code, message, data) = refusal(&response);
        assert_eq!(
            (code, message.as_str(), &data),
            (
                INVALID_PARAMS,
                "lines、heading、outline 一次只能给一个",
                &Value::Null
            ),
            "{part}: {response}"
        );
    }
    let part = text(
        &f,
        &session,
        SMALL,
        json!({"lines": "1-1", "outline": false}),
    )
    .await;
    assert_eq!(after_head(&part).1, "# 一\n");
    assert_eq!(recorded(&f, "content.read").len(), 1);
}

#[tokio::test]
async fn the_tool_schema_offers_the_three_optional_arguments() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    let (_, listed) = f
        .rpc(&f.token, Some(&session), 4, "tools/list", json!({}))
        .await;
    let tool = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "read_note")
        .unwrap();
    let mut names = tool["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["heading", "lines", "outline", "path"]);
    assert_eq!(tool["inputSchema"]["required"], json!(["path"]));
    let handshake = f.call(&f.token, &session, 5, "handshake", json!({})).await;
    assert!(tool_text(&handshake).contains("lines=a-b"));
}

#[tokio::test]
async fn in_replica_mode_the_freshness_line_still_comes_first() {
    let f = support::fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    let path = "project/note.md";
    let whole = f
        .call(&f.token, &session, 2, "read_note", json!({"path": path}))
        .await;
    let whole = support::tool_text(&whole);
    let mut rows = whole.splitn(3, '\n');
    let (revision, freshness, body) = (
        rows.next().unwrap(),
        rows.next().unwrap(),
        rows.next().unwrap(),
    );
    assert!(freshness.contains("generation="), "{whole}");
    let total = body.split_inclusive('\n').count();
    let part = f
        .call(
            &f.token,
            &session,
            3,
            "read_note",
            json!({"path": path, "lines": format!("1-{total}")}),
        )
        .await;
    assert!(!support::is_error(&part), "{part}");
    let part = support::tool_text(&part);
    let mut rows = part.splitn(4, '\n');
    assert_eq!(rows.next().unwrap(), revision);
    assert!(rows.next().unwrap().contains("generation="), "{part}");
    assert_eq!(
        rows.next().unwrap(),
        format!("# 行 1-{total} · 共 {total} 行")
    );
    assert_eq!(rows.next().unwrap(), body);
}
