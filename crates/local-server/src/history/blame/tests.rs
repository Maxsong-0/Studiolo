use super::*;
use studiolo_vault_broker::{HistoryEntry, HistoryLineageEntry, MutationActor};

/// 版本链按最新优先给；`created` 是算创建的那几版的下标。
fn walk(current: &str, newest: bool, chain: &[&str], created: &[usize]) -> Vec<Origin> {
    attribute(
        current,
        newest,
        chain.len(),
        |n| created.contains(&n),
        |n| Ok::<_, ()>(chain[n].to_owned()),
    )
    .unwrap()
}

#[test]
fn each_line_goes_to_the_version_that_first_wrote_it() {
    // 第 2 版（AI 创建）写了三行；第 1 版（人）改了中间一行；第 0 版（AI）在末尾加一行。
    let chain = [
        "标题\n人改的\n结尾\nAI 追加\n",
        "标题\n人改的\n结尾\n",
        "标题\n原来的\n结尾\n",
    ];
    assert_eq!(
        walk(chain[0], true, &chain, &[2]),
        vec![
            Origin::Version(2),
            Origin::Version(1),
            Origin::Version(2),
            Origin::Version(0),
        ]
    );
}

#[test]
fn lines_that_outlive_the_oldest_kept_version_are_not_given_to_its_author() {
    // 最早留下的一版不是创建（更早的已按保留策略清理）：它带着的行归不到任何人。
    let chain = ["旧行\n新行\n", "旧行\n"];
    assert_eq!(
        walk(chain[0], true, &chain, &[]),
        vec![Origin::BeyondHistory, Origin::Version(0)]
    );
    // 一条历史都没有的文档：整篇都查不到。
    assert_eq!(
        walk("甲\n乙\n", false, &[], &[]),
        vec![Origin::BeyondHistory, Origin::BeyondHistory]
    );
}

#[test]
fn a_current_text_missing_from_history_marks_only_its_own_lines_untracked() {
    let chain = ["甲\n乙\n", "甲\n"];
    assert_eq!(
        walk("甲\n库外加的\n乙\n", false, &chain, &[1]),
        vec![Origin::Version(1), Origin::Untracked, Origin::Version(0)]
    );
}

#[test]
fn a_creation_stops_the_walk_even_when_older_entries_remain() {
    // 第 1 版是删掉重建：再往前的同名旧文档里即使有一样的行，也不算进来。
    let chain = ["一样的行\n", "一样的行\n", "一样的行\n"];
    assert_eq!(walk(chain[0], true, &chain, &[1]), vec![Origin::Version(1)]);
}

#[test]
fn matched_lines_are_always_identical_and_keep_their_order() {
    // 重复行、整块搬动、没有任何唯一行的段落：三条路径（头尾、锚点、最长公共子序列）都走到。
    let newer = "a\nx\nx\nb\nc\nx\nd\ny\ny\n";
    let older = "x\nb\na\nx\nx\nc\ny\nd\ny\n";
    let (n, o): (Vec<&str>, Vec<&str>) = (newer.lines().collect(), older.lines().collect());
    let mut budget = WORK_BUDGET;
    let matched = carry::<()>(&n, &o, &mut budget).unwrap();
    let mut last = None;
    let mut count = 0;
    for (line, to) in matched.iter().enumerate() {
        if let Some(to) = to {
            assert_eq!(n[line], o[*to], "只能把内容相同的行认作同一行");
            assert!(last.is_none_or(|previous| previous < *to), "次序不能交叉");
            last = Some(*to);
            count += 1;
        }
    }
    assert!(count >= 4, "至少认出 x、x、c、y 这样一组，实际 {count}");
    // 没有唯一行的小段靠最长公共子序列。
    let (n, o) = (vec!["p", "q", "p", "q"], vec!["q", "p", "q", "p"]);
    let matched = carry::<()>(&n, &o, &mut budget).unwrap();
    assert_eq!(matched.iter().flatten().count(), 3);
}

#[test]
fn limits_fail_closed_instead_of_returning_a_partial_answer() {
    let huge = "行\n".repeat(MAX_LINES + 1);
    assert_eq!(
        attribute(&huge, true, 0, |_| false, |_| Ok::<_, ()>(String::new())),
        Err(Failure::TooLarge)
    );
    let mut budget = 3;
    assert_eq!(
        carry::<()>(&["a", "b", "c"], &["x", "y", "z"], &mut budget),
        Err(Failure::TooLarge)
    );
    assert_eq!(
        attribute("甲\n", false, 1, |_| false, |_| Err::<String, _>("读不到")),
        Err(Failure::Read("读不到"))
    );
}

#[test]
fn response_merges_neighbouring_lines_and_never_invents_an_author() {
    let vault = VaultUuid::new();
    let path = VaultPath::parse("notes/a.md").unwrap();
    let content = "一\n二\n三\n四\n";
    let revision = Revision::compute(vault, &path, content.as_bytes());
    let older = Revision::compute(vault, &path, b"older");
    let entry = |id: &str, revision, author: Option<MutationActor>| HistoryLineageEntry {
        entry: HistoryEntry {
            id: id.repeat(64),
            revision,
            bytes: 1,
            authored_at_ms: author.as_ref().map(|_| 1_700_000_000_000),
            author: author.clone(),
            archived_at_ms: Some(1_700_000_000_001),
            archived_by: author,
        },
        created: false,
    };
    let lineage = HistoryLineage {
        entries: vec![
            entry("a", revision, Some(MutationActor::Obsidian)),
            entry("b", older, None),
        ],
        more: true,
    };
    let origins = [
        Origin::Version(0),
        Origin::Version(0),
        Origin::Version(1),
        Origin::BeyondHistory,
    ];
    let value = response(vault, &path, revision, content, &lineage, &origins);
    assert_eq!(value["lines"], 4);
    assert_eq!(value["versions"], 2);
    assert_eq!(value["more_history"], true);
    assert_eq!(value["revision"], revision.to_hex());
    assert_eq!(value["content_sha256"], content_digest(content));
    let runs = value["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        (&runs[0]["from_line"], &runs[0]["to_line"]),
        (&json!(1), &json!(2))
    );
    assert_eq!(runs[0]["origin"], "version");
    assert_eq!(runs[0]["author"]["kind"], "obsidian");
    assert_eq!(runs[0]["history_id"], "a".repeat(64));
    // 版本在、主体未知：照实给 null，不拿归档的人顶上。
    assert_eq!(runs[1]["origin"], "version");
    assert!(runs[1]["author"].is_null() && runs[1]["authored_at_ms"].is_null());
    assert_eq!(runs[1]["revision"], older.to_hex());
    assert_eq!(runs[2]["origin"], "beyond_history");
    assert!(runs[2]["history_id"].is_null() && runs[2]["revision"].is_null());
    assert_eq!(
        content_digest(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}
