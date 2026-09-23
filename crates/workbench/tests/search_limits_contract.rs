//! 经公开搜索接口验证查询、摘录与结果条数的预算。

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
fn long_unicode_line_snippet_is_bounded_and_contains_the_first_match() {
    let line = format!(
        "{}关键（匹配）{}关键（匹配）",
        "前文".repeat(1_000),
        "🪴后文".repeat(1_000)
    );
    let model = WorkbenchReadModel::build(&[note("长行.md", line)]).unwrap();
    let result = model
        .search(&query("关键（匹配）", SearchMode::FullText))
        .unwrap();
    assert_eq!(result.hits.len(), 1);
    assert!(result.hits[0].snippet.chars().count() <= 320);
    assert!(result.hits[0].snippet.contains("前文关键（匹配）🪴后文"));
    assert_eq!(result.hits[0].snippet.matches("关键（匹配）").count(), 1);
}

#[test]
fn a_maximum_length_literal_is_accepted_and_fits_inside_its_snippet() {
    let text = "界".repeat(256);
    let model = WorkbenchReadModel::build(&[note(
        "长查询.md",
        format!("{}{}{}", "前".repeat(500), text, "后".repeat(500)),
    )])
    .unwrap();
    let result = model.search(&query(&text, SearchMode::FullText)).unwrap();
    assert_eq!(result.hits.len(), 1);
    assert!(result.hits[0].snippet.contains(&text));
    assert!(result.hits[0].snippet.chars().count() <= 320);
}

#[test]
fn long_titles_cannot_amplify_many_hits_and_full_paths_remain_intact() {
    let path = format!("资料/{}.md", "路".repeat(400));
    let content = format!(
        "# {}\n{}",
        "长标题🪴".repeat(10_000),
        "needle\n".repeat(101)
    );
    let model = WorkbenchReadModel::build(&[note(&path, content)]).unwrap();
    let result = model
        .search(&query("needle", SearchMode::FullText))
        .unwrap();
    assert_eq!(result.hits.len(), 100);
    assert!(result.truncated);
    for hit in &result.hits {
        assert_eq!(hit.path, path);
        assert_eq!(hit.title.chars().count(), 320);
        assert!(hit.title.ends_with('…'));
        assert!(hit.snippet.chars().count() <= 320);
    }
}

#[test]
fn titles_at_the_limit_are_not_falsely_marked_truncated() {
    for length in [319, 320, 321] {
        let title = "界".repeat(length);
        let model =
            WorkbenchReadModel::build(&[note("title.md", format!("# {title}\nneedle"))]).unwrap();
        let result = model
            .search(&query("needle", SearchMode::FullText))
            .unwrap();
        assert_eq!(result.hits[0].title.chars().count(), length.min(320));
        assert_eq!(result.hits[0].title.ends_with('…'), length > 320);
    }
}

#[test]
fn exactly_one_hundred_results_are_not_marked_truncated_but_one_hundred_one_are() {
    let mut documents = (0..100)
        .map(|index| note(&format!("资料/{index:03}.md"), "body"))
        .collect::<Vec<_>>();
    let exact = WorkbenchReadModel::build(&documents).unwrap();
    let exact = exact.search(&query("", SearchMode::Path)).unwrap();
    assert_eq!(exact.hits.len(), 100);
    assert!(!exact.truncated);
    documents.push(note("资料/100.md", "body"));
    let extra = WorkbenchReadModel::build(&documents).unwrap();
    let extra = extra.search(&query("", SearchMode::Path)).unwrap();
    assert_eq!(extra.hits, exact.hits);
    assert!(extra.truncated);
}

#[test]
fn result_limit_also_applies_to_many_matching_lines_in_one_document() {
    for lines in [100, 101] {
        let model =
            WorkbenchReadModel::build(&[note("many.md", "needle\n".repeat(lines))]).unwrap();
        let result = model
            .search(&query("needle", SearchMode::FullText))
            .unwrap();
        assert_eq!(result.hits.len(), 100);
        assert_eq!(result.hits[99].line, 100);
        assert_eq!(result.truncated, lines == 101);
    }
}

#[test]
fn results_are_stable_by_path_then_line_with_path_hit_before_content() {
    let a = note("a.md", "needle\nneedle");
    let b = note("b-needle.md", "needle\nneedle");
    let forward = WorkbenchReadModel::build(&[a.clone(), b.clone()]).unwrap();
    let backward = WorkbenchReadModel::build(&[b, a]).unwrap();
    let request = query("needle", SearchMode::FullText);
    let result = forward.search(&request).unwrap();
    assert_eq!(result, backward.search(&request).unwrap());
    let positions = result
        .hits
        .iter()
        .map(|hit| (hit.path.as_str(), hit.line, hit.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        positions,
        vec![
            ("a.md", 1, SearchHitKind::Content),
            ("a.md", 2, SearchHitKind::Content),
            ("b-needle.md", 1, SearchHitKind::Path),
            ("b-needle.md", 1, SearchHitKind::Content),
            ("b-needle.md", 2, SearchHitKind::Content)
        ]
    );
}

#[test]
fn oversized_or_control_character_queries_are_rejected_before_search() {
    let model = WorkbenchReadModel::build(&[]).unwrap();
    for invalid in [
        "x".repeat(257),
        "界".repeat(257),
        "😀".repeat(257),
        "a\nb".into(),
        "a\rb".into(),
        "a\tb".into(),
        "\0".into(),
        "\u{7f}".into(),
    ] {
        for mode in [SearchMode::FullText, SearchMode::Path] {
            assert!(matches!(
                model.search(&query(&invalid, mode)),
                Err(SearchError::InvalidQuery)
            ));
        }
    }
    let mut invalid_project = query("", SearchMode::FullText);
    invalid_project.project = Some("项目/不存在".into());
    assert!(matches!(
        model.search(&invalid_project),
        Err(SearchError::InvalidProject)
    ));
}
