//! T-210：逐行归属和变更水位的真实会话、真实 Broker 闭环。
use crate::{
    editing::{body, refresh, save},
    fixture::Fixture,
};
use serde_json::Value;
use std::fs;
use studiolo_core::VaultUuid;

const BLAME: &str = "/api/v1/history/blame?path=a.md";
const HEAD: &str = "/api/v1/changes/head";

/// 每一行的归属摊开：(origin, 作者 subject 或空, 那一版的 revision 或空)。
fn per_line(blame: &Value) -> Vec<(String, String, String)> {
    let mut lines = Vec::new();
    for run in blame["runs"].as_array().unwrap() {
        let (from, to) = (
            run["from_line"].as_u64().unwrap(),
            run["to_line"].as_u64().unwrap(),
        );
        assert_eq!(from as usize, lines.len() + 1, "各段必须首尾相接、不留空行");
        assert!(to >= from);
        for _ in from..=to {
            lines.push((
                run["origin"].as_str().unwrap().to_owned(),
                run["author"]["subject"].as_str().unwrap_or("").to_owned(),
                run["revision"].as_str().unwrap_or("").to_owned(),
            ));
        }
    }
    assert_eq!(lines.len() as u64, blame["lines"].as_u64().unwrap());
    lines
}

#[tokio::test]
async fn blame_gives_each_line_to_its_first_version_and_never_guesses() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let original = String::from_utf8(f.expected.content.clone()).unwrap();
    let base = original.trim_end_matches('\n');
    let base_lines = base.lines().count();
    assert!(base_lines >= 1, "夹具文档至少一行");

    // 一条历史都没有：整篇早于可查历史，不归给任何人。
    let untouched = f.get(BLAME, &issued.cookie, 200).await;
    assert_eq!(untouched["vault_uuid"], f.expected.vault_uuid.to_string());
    assert_eq!(untouched["path"], "a.md");
    assert_eq!(untouched["revision"], f.expected.revision.to_hex());
    assert_eq!(untouched["versions"], 0);
    assert!(
        per_line(&untouched)
            .iter()
            .all(|line| line.0 == "beyond_history" && line.1.is_empty())
    );

    // 人先后各加一行。
    let first_text = format!("{base}\n人加的第一行\n");
    let first = save(
        &f,
        &issued,
        &body(&f, &first_text, "blame-first-save-001"),
        200,
    )
    .await["receipt"]["new_revision"]
        .as_str()
        .unwrap()
        .to_owned();
    refresh(&f, &issued).await;
    let second_text = format!("{first_text}人加的第二行\n");
    let mut input = body(&f, &second_text, "blame-second-save-002");
    input["expected_revision"] = Value::String(first.clone());
    let second = save(&f, &issued, &input, 200).await["receipt"]["new_revision"]
        .as_str()
        .unwrap()
        .to_owned();
    refresh(&f, &issued).await;

    let blame = f
        .get(
            &format!("{BLAME}&vault_uuid={}", f.expected.vault_uuid),
            &issued.cookie,
            200,
        )
        .await;
    assert_eq!(blame["revision"], second);
    assert_eq!(blame["content_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(blame["more_history"], false);
    let lines = per_line(&blame);
    assert_eq!(lines.len(), base_lines + 2);
    // 库里原来就有的正文：作者未知，也不是这两次保存创建的。
    assert!(
        lines[..base_lines]
            .iter()
            .all(|line| line.0 == "beyond_history" && line.1.is_empty() && line.2.is_empty())
    );
    assert_eq!(
        lines[base_lines],
        ("version".into(), "fictional-person".into(), first.clone())
    );
    assert_eq!(
        lines[base_lines + 1],
        ("version".into(), "fictional-person".into(), second.clone())
    );

    // 库外直接改文件：新加的那一行没有记录，别的行归属不变。
    fs::write(
        f.http.root.join("vault/a.md"),
        format!("{second_text}库外加的一行\n"),
    )
    .unwrap();
    refresh(&f, &issued).await;
    let outside = per_line(&f.get(BLAME, &issued.cookie, 200).await);
    assert_eq!(outside.len(), base_lines + 3);
    assert_eq!(outside[base_lines + 2].0, "untracked");
    assert!(outside[base_lines + 2].1.is_empty() && outside[base_lines + 2].2.is_empty());
    assert_eq!(outside[..base_lines + 2], lines[..]);

    // 失败关闭：没有会话、库对不上、保护路径、不是 Markdown、多余字段。
    f.http
        .send(f.http.request("GET", BLAME, None, ""), 401)
        .await;
    f.get(
        &format!("{BLAME}&vault_uuid={}", VaultUuid::new()),
        &issued.cookie,
        403,
    )
    .await;
    f.get("/api/v1/history/blame?path=CLAUDE.md", &issued.cookie, 403)
        .await;
    for bad in [
        "/api/v1/history/blame?path=a.txt",
        "/api/v1/history/blame",
        "/api/v1/history/blame?path=a.md&limit=1",
        "/api/v1/history/blame?path=a.md&path=a.md",
    ] {
        assert_eq!(
            f.get(bad, &issued.cookie, 400).await["code"],
            "invalid_history_query",
            "{bad}"
        );
    }
    f.close().await;
}

#[tokio::test]
async fn changes_head_is_the_list_watermark_and_carries_no_entries() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    f.http
        .send(f.http.request("GET", HEAD, None, ""), 401)
        .await;
    // 后台例行检查可能正好在两次读取之间发布新代次：前后两次水位相同才比列表。
    let mut compared = false;
    for _ in 0..5 {
        let before = f.get(HEAD, &issued.cookie, 200).await;
        let page = f.get("/api/v1/changes", &issued.cookie, 200).await;
        let after = f.get(HEAD, &issued.cookie, 200).await;
        if before != after {
            continue;
        }
        assert_eq!(
            before["through"], page["through"],
            "水位和列表签发的是同一组数"
        );
        assert_eq!(before["vault_uuid"], page["vault_uuid"]);
        assert_eq!(before.as_object().unwrap().len(), 2, "只有库身份和水位");
        assert_eq!(before["through"].as_object().unwrap().len(), 4);
        assert!(before.to_string().len() <= 512);
        compared = true;
        break;
    }
    assert!(compared, "五次都撞上发布，水位比不了");
    let start = f.get(HEAD, &issued.cookie, 200).await;
    assert_eq!(start["through"]["sequence"], "0");

    // 经 Broker 的保存推进提交序号。
    save(
        &f,
        &issued,
        &body(&f, "# 水位\n", "head-watermark-save-001"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let saved = f.get(HEAD, &issued.cookie, 200).await;
    assert_eq!(saved["through"]["sequence"], "1");
    assert_eq!(saved["through"]["epoch"], start["through"]["epoch"]);

    // 库外的改动不动提交序号，只推进观察序号。
    fs::create_dir_all(f.http.root.join("vault/项目/Alpha")).unwrap();
    fs::write(f.http.root.join("vault/项目/Alpha/external.md"), "# 外部\n").unwrap();
    refresh(&f, &issued).await;
    let observed = f.get(HEAD, &issued.cookie, 200).await;
    assert_eq!(observed["through"]["sequence"], "1");
    assert_ne!(
        observed["through"]["observation"],
        saved["through"]["observation"]
    );

    f.get(
        &format!("{HEAD}?vault_uuid={}", f.expected.vault_uuid),
        &issued.cookie,
        200,
    )
    .await;
    f.get(
        &format!("{HEAD}?vault_uuid={}", VaultUuid::new()),
        &issued.cookie,
        403,
    )
    .await;
    assert_eq!(
        f.get(&format!("{HEAD}?limit=1"), &issued.cookie, 400).await["code"],
        "invalid_changes_query"
    );
    f.close().await;
}
