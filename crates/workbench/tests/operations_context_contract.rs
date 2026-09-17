//! 确认模糊条目与 Markdown 上下文不会制造未文档化目录或选错章节。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::WorkbenchReadModel;

fn operations(content: &str) -> studiolo_workbench::ProjectOperations {
    WorkbenchReadModel::build(&[MarkdownDocument::new(
        VaultPath::parse("项目/甲/01-框架/目录结构.md").unwrap(),
        content,
    )])
    .unwrap()
    .project_operations("项目/甲")
    .unwrap()
}

#[test]
fn ambiguous_visible_file_rows_cannot_leave_a_partial_comparison_scope() {
    for entry in [
        "scripts/a.py / scripts/b.py",
        "...",
        "src/…",
        "scripts/*.py",
        "../outside.py",
    ] {
        let content = format!("## 代码仓库\n```text\nrepo/\n├── web/\n└── {entry}\n```\n");
        let result = operations(&content);
        assert_eq!(result.declaration_status, "invalid", "{entry}");
        assert!(result.directories.is_empty());
        assert!(result.comparison_parents.is_empty());
    }
}

#[test]
fn ordinary_spaced_files_and_hidden_files_keep_only_their_visible_parents() {
    let result = operations(
        "## 代码仓库\n```text\nrepo/\n├── web/\n├── source files/my file.rs\n├── docs/.cache/file.txt\n└── .github/workflows/ci.yml\n```\n",
    );
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.comparison_parents, [""]);
    assert_eq!(
        result
            .directories
            .iter()
            .map(|node| node.path.as_str())
            .collect::<Vec<_>>(),
        ["docs", "source files", "web"]
    );
    assert!(
        result
            .directories
            .iter()
            .filter(|node| node.path != "web")
            .all(|node| node.source.is_none())
    );
}

#[test]
fn comment_openers_inside_fences_do_not_hide_a_later_real_repository_section() {
    let result = operations(
        "```md\n<!--\n## 代码仓库\n```\n## 代码仓库\n```text\nrepo/\n└── actual/\n```\n",
    );
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.directories[0].path, "actual");
    assert_eq!(result.declaration_source.as_ref().unwrap().line, 6);
    assert_eq!(result.directories[0].source.as_ref().unwrap().line, 8);
}

#[test]
fn inline_html_comments_mask_fake_headings_and_fences_until_their_actual_close() {
    let prefix = "说明 <!--\n## 代码仓库\n```text\nrepo/\n└── fake/\n```\n-->\n";
    let absent = operations(prefix);
    assert_eq!(absent.declaration_status, "unavailable");
    let content = format!("{prefix}## 代码仓库\n```text\nrepo/\n└── actual/\n```\n");
    let result = operations(&content);
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.directories[0].path, "actual");
    assert_eq!(result.declaration_source.as_ref().unwrap().line, 9);
    assert_eq!(result.directories[0].source.as_ref().unwrap().line, 11);
}

#[test]
fn comments_inside_a_repository_section_cannot_replace_its_first_real_tree() {
    let result = operations(
        "## 代码仓库\n说明 <!--\n```text\nrepo/\n└── fake/\n```\n-->\n```text\nrepo/\n└── actual/\n```\n",
    );
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.directories[0].path, "actual");
    assert_eq!(result.declaration_source.as_ref().unwrap().line, 8);
}

#[test]
fn inline_code_comment_literals_and_real_heading_annotations_remain_supported() {
    let result = operations(
        "说明 `<!--` 与 `` `<!--` `` 均是代码。\n## 代码仓库 <!-- 当前结构 -->\n```text\nrepo/\n└── actual/ # <!-- 原样职责 -->\n```\n",
    );
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.directories[0].label, "<!-- 原样职责 -->");
    assert_eq!(result.directories[0].source.as_ref().unwrap().line, 5);
}

#[test]
fn removing_a_comment_cannot_create_a_new_heading_or_fence_on_the_same_line() {
    let absent = operations("<!-- x --> ## 代码仓库\n```text\nrepo/\n└── fake/\n```\n");
    assert_eq!(absent.declaration_status, "unavailable");
    let result = operations("## 代码仓库\n<!-- x --> ```text\nrepo/\n└── fake/\n## 下一章节\n");
    assert_eq!(result.declaration_status, "unavailable");
}
