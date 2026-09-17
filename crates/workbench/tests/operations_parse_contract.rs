//! 图 2 首片的范围、来源与保守解析契约。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{ProjectOperations, WorkbenchReadModel};

const PATH: &str = "项目/Studiolo/01-框架/目录结构.md";

fn model(content: &str) -> WorkbenchReadModel {
    WorkbenchReadModel::build(&[MarkdownDocument::new(
        VaultPath::parse(PATH).unwrap(),
        content,
    )])
    .unwrap()
}

fn operations(content: &str) -> ProjectOperations {
    model(content).project_operations("项目/Studiolo").unwrap()
}

fn tree(content: &str) -> String {
    format!("# 目录结构\n## 代码仓库\n```text\nstudiolo/\n{content}\n```\n")
}

#[test]
fn studiolo_sample_preserves_repository_scope_responsibilities_and_sources() {
    let content = "# 目录结构\n## 档案（本 vault）\n```text\n项目/Studiolo/\n└── 00-总览/\n```\n## 代码仓库（T-041）\n```text\nstudiolo/\n├── Cargo.toml # workspace\n├── crates/ # 共享模块\n│   ├── core/ # 共享 ID、路径\n│   └── workbench/ # 纯派生 <b>视图</b> & 职责\n├── docs/contracts/ # 接口契约\n├── docs/m1/ # 人侧监督\n├── scripts/test-linux.sh # 验证脚本\n├── .github/workflows/ci.yml # 隐藏文件不供给\n└── web/ # 中文界面\n```\n## 服务器部署\n```text\nreleases/\n└── private/\n```\n";
    let value = operations(content);
    assert_eq!(value.declaration_status, "ready");
    assert_eq!(value.declaration_source.as_ref().unwrap().line, 8);
    assert_eq!(value.comparison_parents, ["", "crates", "docs"]);
    let paths = value
        .directories
        .iter()
        .map(|directory| directory.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "crates",
            "crates/core",
            "crates/workbench",
            "docs",
            "docs/contracts",
            "docs/m1",
            "scripts",
            "web"
        ]
    );
    let declared = value
        .directories
        .iter()
        .find(|directory| directory.path == "crates/workbench")
        .unwrap();
    assert_eq!(declared.label, "纯派生 <b>视图</b> & 职责");
    assert_eq!(declared.source.as_ref().unwrap().path, PATH);
    assert_eq!(declared.source.as_ref().unwrap().line, 13);
    assert!(
        value
            .directories
            .iter()
            .filter(|directory| ["docs", "scripts"].contains(&directory.path.as_str()))
            .all(|directory| directory.source.is_none())
    );
}

#[test]
fn missing_or_unknown_project_never_looks_like_a_successful_check() {
    let model = model("# 目录结构\n## 档案\n```text\nrepo/\n└── not-code/\n```\n");
    assert!(model.project_operations("项目/Studiolo-副本").is_none());
    let unavailable = model.project_operations("项目/Studiolo").unwrap();
    assert_eq!(unavailable.declaration_status, "unavailable");
    assert!(unavailable.directories.is_empty());
    assert!(!unavailable.diagnostics.is_empty());
    let model = WorkbenchReadModel::build(&[MarkdownDocument::new(
        VaultPath::parse("项目/无目录/00-总览/INDEX.md").unwrap(),
        "# 项目",
    )])
    .unwrap();
    let unavailable = model.project_operations("项目/无目录").unwrap();
    assert_eq!(unavailable.declaration_status, "unavailable");
    assert!(unavailable.declaration_source.is_none());
}

#[test]
fn only_first_text_fence_in_real_level_two_repository_section_is_selected() {
    let content = "---\nexample: |\n  ## 代码仓库\n---\n<!--\n## 代码仓库\n-->\n```md\n## 代码仓库\n```\n### 代码仓库\n```text\nwrong/\n└── wrong/\n```\n## 代码仓库\n<!--\n```text\nwrong/\n└── hidden/\n```\n-->\n```rust\nlet text = 1;\n```\n~~~~text\nrepo/\n└── 正确/\n~~~~\n```text\nrepo/\n└── second/\n```\n";
    let value = operations(content);
    assert_eq!(value.declaration_status, "ready");
    assert_eq!(value.directories[0].path, "正确");
    assert_eq!(value.directories.len(), 1);
}

#[test]
fn broken_or_ambiguous_directory_declarations_reject_the_whole_block() {
    for invalid in [
        "├── valid/\n    └── child/\n        └── too-deep/\n                └── jump/",
        "├── README.md\n│   └── impossible/",
        "├── valid/\n  └── bad-indent/",
        "├── valid/\n└── ../escape/",
        "├── valid/\n└── .private/",
        "├── valid/\n└── .../",
        "├── valid/\n└── PRD-01…PRD-10/",
        "├── valid/\n└── first/ / second/",
        "├── valid/\n└── wildcard*/",
        "├── valid/\n└── broken\\path/",
        "├── valid/\n└── invisible\u{200b}/",
        "├── valid/\n└── valid/",
        "├── Upper/\n└── upper/",
    ] {
        let value = operations(&tree(invalid));
        assert_eq!(value.declaration_status, "invalid", "{invalid}");
        assert!(value.directories.is_empty(), "{invalid}");
        assert!(value.comparison_parents.is_empty(), "{invalid}");
        assert!(!value.diagnostics.is_empty(), "{invalid}");
    }
}

#[test]
fn implicit_parents_can_gain_one_explicit_declaration_without_expanding_file_only_scopes() {
    let value = operations(&tree(
        "├── scripts/tools/check.py\n├── docs/api/ # API 说明\n└── docs/ # 文档职责",
    ));
    assert_eq!(value.declaration_status, "ready");
    assert_eq!(value.comparison_parents, ["", "docs"]);
    let docs = value
        .directories
        .iter()
        .find(|directory| directory.path == "docs")
        .unwrap();
    assert_eq!(docs.label, "文档职责");
    assert!(docs.source.is_some());
    for implicit in ["scripts", "scripts/tools"] {
        assert!(
            value
                .directories
                .iter()
                .find(|directory| directory.path == implicit)
                .unwrap()
                .source
                .is_none()
        );
    }
}

#[test]
fn chinese_spaced_paths_and_plain_text_labels_are_retained_without_html_interpretation() {
    let value = operations(&tree("└── 中文 目录/ # <svg onload=\"bad\"> & `原样标签`"));
    assert_eq!(value.declaration_status, "ready");
    assert_eq!(value.directories[0].path, "中文 目录");
    assert_eq!(
        value.directories[0].label,
        "<svg onload=\"bad\"> & `原样标签`"
    );
}

#[test]
fn block_and_directory_limits_degrade_atomically_without_a_partial_green_tree() {
    let too_large = format!("└── src/ # {}", "中".repeat(23_000));
    let too_many = (0..257)
        .map(|index| format!("├── d{index}/"))
        .collect::<Vec<_>>()
        .join("\n");
    let too_many_parents = (0..65)
        .map(|index| format!("├── d{index}/child/"))
        .collect::<Vec<_>>()
        .join("\n");
    for content in [too_large, too_many, too_many_parents] {
        let value = operations(&tree(&content));
        assert_eq!(value.declaration_status, "invalid");
        assert!(value.directories.is_empty());
        assert!(value.comparison_parents.is_empty());
    }
    let at_limit = (0..256)
        .map(|index| format!("├── d{index}/"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(operations(&tree(&at_limit)).directories.len(), 256);
}

#[test]
fn unclosed_fence_wrong_root_and_empty_block_are_not_accepted_as_empty_repositories() {
    for content in [
        "## 代码仓库\n```text\nrepo/\n└── src/",
        "## 代码仓库\n```text\n/absolute/\n└── src/\n```",
        "## 代码仓库\n```text\nrepo/nested/\n└── src/\n```",
        "## 代码仓库\n```text\n```",
    ] {
        assert_eq!(operations(content).declaration_status, "invalid");
    }
}

#[test]
fn rebuilding_after_rename_replaces_the_old_directory_declaration() {
    let old = operations(&tree("└── old/ # 旧职责"));
    let new = operations(&tree("└── new/ # 新职责"));
    assert_eq!(old.directories[0].path, "old");
    assert_eq!(new.directories[0].path, "new");
    assert_eq!(new.directories.len(), 1);
}

#[test]
fn a_single_deep_path_is_rejected_during_parent_expansion_without_a_partial_tree() {
    let deep = std::iter::repeat_n("深", 4096)
        .collect::<Vec<_>>()
        .join("/");
    // 两行都小于声明字节上限：目录触发父层限制，文件隐含父路径触发目录数限制。
    for entry in [format!("{deep}/"), format!("{deep}/file.rs")] {
        let content = tree(&format!("├── valid/\n└── {entry}"));
        assert!(content.len() < 64 * 1024);
        let result = operations(&content);
        assert_eq!(result.declaration_status, "invalid");
        assert!(result.directories.is_empty());
        assert!(result.comparison_parents.is_empty());
    }
    let boundary = std::iter::repeat_n("层", 64).collect::<Vec<_>>().join("/");
    let result = operations(&tree(&format!("└── {boundary}/")));
    assert_eq!(result.declaration_status, "ready");
    assert_eq!(result.directories.len(), 64);
    assert_eq!(result.comparison_parents.len(), 64);
}
