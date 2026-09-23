//! T-007 测试走的是与 `HTTP` 调用方相同的内存搜索接口。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{SearchError, SearchHitKind, SearchMode, SearchQuery, WorkbenchReadModel};

fn note(path: &str, content: impl Into<String>) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

fn query(text: &str, mode: SearchMode) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        project: None,
        mode,
    }
}

#[test]
fn chinese_substrings_match_across_punctuation_with_exact_source_lines() {
    let model = WorkbenchReadModel::build(&[note(
        "资料/阅读.md",
        "# 阅读\n这里是设计评审：本机只读，待验收。\n不相关正文",
    )])
    .unwrap();
    let result = model
        .search(&query("评审：本机", SearchMode::FullText))
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    let hit = &result.hits[0];
    assert_eq!(hit.path, "资料/阅读.md");
    assert_eq!(hit.title, "阅读");
    assert_eq!(hit.line, 2);
    assert_eq!(hit.kind, SearchHitKind::Content);
    assert!(hit.snippet.contains("评审：本机"));
    assert!(!result.truncated);
}

#[test]
fn fulltext_search_includes_full_paths_and_is_case_insensitive() {
    let model = WorkbenchReadModel::build(&[note(
        "项目/Alpha/Notes/PLAN.md",
        "# 计划\nSome NeEdLe content",
    )])
    .unwrap();
    let paths = model
        .search(&query("alpha/notes/plan.MD", SearchMode::FullText))
        .unwrap();
    assert_eq!(paths.hits.len(), 1);
    assert_eq!(paths.hits[0].kind, SearchHitKind::Path);
    assert_eq!(paths.hits[0].line, 1);
    let contents = model
        .search(&query("nEeDlE", SearchMode::FullText))
        .unwrap();
    assert_eq!(contents.hits.len(), 1);
    assert_eq!(contents.hits[0].line, 2);
}

#[test]
fn regex_metacharacters_are_literal_in_both_modes() {
    let model =
        WorkbenchReadModel::build(&[note("资料/a+b[1].md", "literal a+b?.*[x] end\naaaaaabZx")])
            .unwrap();
    let literal = model
        .search(&query("a+b?.*[x]", SearchMode::FullText))
        .unwrap();
    assert_eq!(literal.hits.len(), 1);
    assert_eq!(literal.hits[0].line, 1);
    let path = model.search(&query("a+b[1]", SearchMode::Path)).unwrap();
    assert_eq!(path.hits.len(), 1);
    assert_eq!(path.hits[0].kind, SearchHitKind::Path);
}

#[test]
fn project_filter_is_an_exact_known_id_with_a_directory_separator() {
    let model = WorkbenchReadModel::build(&[
        note("项目/甲/笔记.md", "共同词"),
        note("项目/甲乙/笔记.md", "共同词"),
        note("其他/甲/笔记.md", "共同词"),
    ])
    .unwrap();
    let mut selected = query("共同词", SearchMode::FullText);
    selected.project = Some("项目/甲".into());
    let result = model.search(&selected).unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].path, "项目/甲/笔记.md");
    for invalid in ["甲", "项目/甲/", "项目/不存在", "项目"] {
        selected.project = Some(invalid.into());
        assert!(matches!(
            model.search(&selected),
            Err(SearchError::InvalidProject)
        ));
    }
}

#[test]
fn path_mode_matches_all_space_separated_fragments_in_any_order_not_content() {
    let model = WorkbenchReadModel::build(&[
        note("项目/Alpha/Notes/PLAN.md", "# 目标"),
        note("项目/Alpha/其他.md", "PLAN Notes"),
        note("项目/Beta/Notes/PLAN.md", "# 其他"),
    ])
    .unwrap();
    let result = model
        .search(&query("  PLAN   alpha/  notes ", SearchMode::Path))
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].path, "项目/Alpha/Notes/PLAN.md");
    assert_eq!(result.hits[0].kind, SearchHitKind::Path);
}

#[test]
fn empty_fulltext_is_empty_but_empty_path_query_lists_documents() {
    let model = WorkbenchReadModel::build(&[note("b.md", "B"), note("a.md", "A")]).unwrap();
    assert!(
        model
            .search(&query("   ", SearchMode::FullText))
            .unwrap()
            .hits
            .is_empty()
    );
    let result = model.search(&query("   ", SearchMode::Path)).unwrap();
    assert_eq!(
        result
            .hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.md", "b.md"]
    );
    assert!(
        result
            .hits
            .iter()
            .all(|hit| hit.line == 1 && hit.kind == SearchHitKind::Path)
    );
    assert!(!result.truncated);
}

#[test]
fn crlf_and_a_final_line_without_newline_preserve_one_based_locations() {
    let model = WorkbenchReadModel::build(&[note("note.md", "标题\r\nneedle one\r\nfinal needle")])
        .unwrap();
    let result = model
        .search(&query("needle", SearchMode::FullText))
        .unwrap();
    assert_eq!(
        result.hits.iter().map(|hit| hit.line).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(result.hits[0].snippet, "needle one");
    assert_eq!(result.hits[1].snippet, "final needle");
}

#[test]
fn path_mode_splits_fragments_on_ideographic_and_other_unicode_whitespace() {
    let model = WorkbenchReadModel::build(&[
        note("项目/纸页/01-框架/方案A/配色方案.md", "# 配色"),
        note("项目/纸页/01-框架/阅读体验.md", "# 阅读"),
    ])
    .unwrap();
    for query_text in [
        "纸页\u{3000}配色",
        "纸页\u{a0}配色",
        " 纸页\u{3000}\u{3000}配色\u{3000}",
    ] {
        let result = model.search(&query(query_text, SearchMode::Path)).unwrap();
        assert_eq!(result.hits.len(), 1, "query {query_text:?}");
        assert_eq!(result.hits[0].path, "项目/纸页/01-框架/方案A/配色方案.md");
    }
}
