//! Public-Interface contracts for bounded, source-linked M1 navigation.

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::WorkbenchReadModel;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn note(value: &str, body: &str) -> MarkdownDocument {
    MarkdownDocument::new(path(value), body)
}

#[test]
fn chinese_heading_links_and_global_backlinks_preserve_exact_source_lines() {
    let model = WorkbenchReadModel::build(&[
        note("项目/甲/入口.md", "\u{feff}---\r\ntitle: 起点\r\n---\r\n# 起点\r\n😀 [[目标#中文标题|别名]] [[目标#missing]] [[#起点]]\r\n![[目标]]\r\n"),
        note("项目/甲/目标.md", "---\naliases: [别的名]\n---\n# 文档标题\n## 中文标题\n"),
        note("库外层入口.md", "[[项目/甲/目标#中文标题|全库入链]]"),
    ]).unwrap();
    let navigation = model.navigation(&path("项目/甲/入口.md")).unwrap();
    assert_eq!(navigation.outline.len(), 1);
    assert_eq!(navigation.outline[0].source.line, 4);
    assert_eq!(navigation.outgoing.len(), 4);
    let first = &navigation.outgoing[0];
    assert_eq!(first.label, "别名");
    assert_eq!(first.raw, "目标#中文标题|别名");
    assert_eq!(first.source.line, 5);
    assert_eq!(first.status, "resolved");
    assert_eq!(first.targets[0].path, "项目/甲/目标.md");
    assert_eq!(first.targets[0].line, 5);
    assert_eq!(navigation.outgoing[1].status, "missing_heading");
    assert!(navigation.outgoing[1].targets.is_empty());
    assert_eq!(navigation.outgoing[2].targets[0].line, 4);
    assert_eq!(navigation.outgoing[3].targets[0].line, 1);
    assert!(!navigation.truncated);
    let target = model.navigation(&path("项目/甲/目标.md")).unwrap();
    assert_eq!(target.backlinks.len(), 4);
    assert!(
        target
            .backlinks
            .iter()
            .any(|link| link.source.path == "库外层入口.md" && link.source.line == 1)
    );
}

#[test]
fn nearest_aliases_and_page_ambiguities_reuse_the_m0_resolution_rules() {
    let model = WorkbenchReadModel::build(&[
        note(
            "项目/甲/日志/今天.md",
            "[[任务板|协调页]] [[项目/丙/任务板]]",
        ),
        note(
            "项目/甲/协调/任务.md",
            "---\naliases: [任务板]\n---\n# 甲任务",
        ),
        note(
            "项目/乙/协调/任务.md",
            "---\naliases: 任务板\n---\n# 乙任务",
        ),
        note("入口.md", "[[任务板]]"),
    ])
    .unwrap();
    let nearest = model.navigation(&path("项目/甲/日志/今天.md")).unwrap();
    assert_eq!(nearest.outgoing[0].status, "resolved");
    assert_eq!(nearest.outgoing[0].label, "协调页");
    assert_eq!(nearest.outgoing[0].targets[0].path, "项目/甲/协调/任务.md");
    assert_eq!(nearest.outgoing[1].status, "missing");
    assert!(nearest.outgoing[1].targets.is_empty());
    let ambiguous = model.navigation(&path("入口.md")).unwrap();
    assert_eq!(ambiguous.outgoing[0].status, "ambiguous");
    assert_eq!(ambiguous.outgoing[0].targets.len(), 2);
    assert!(
        ambiguous.outgoing[0]
            .targets
            .iter()
            .all(|target| target.path.starts_with("项目/") && target.line == 1)
    );
}

#[test]
fn heading_text_precedes_anchor_and_duplicate_text_never_guesses_a_line() {
    let model = WorkbenchReadModel::build(&[
        note(
            "入口.md",
            "[[目标#本周]] [[目标#本周-1]] [[目标#another-heading]] [[目标#缺失]]",
        ),
        note("目标.md", "# 目标\n## 本周\n## 本周\n## Another Heading\n"),
    ])
    .unwrap();
    let navigation = model.navigation(&path("入口.md")).unwrap();
    assert_eq!(navigation.outgoing[0].status, "ambiguous_heading");
    assert_eq!(
        navigation.outgoing[0]
            .targets
            .iter()
            .map(|target| target.line)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(navigation.outgoing[1].status, "resolved");
    assert_eq!(navigation.outgoing[1].targets[0].line, 3);
    assert_eq!(navigation.outgoing[2].targets[0].line, 4);
    assert_eq!(navigation.outgoing[3].status, "missing_heading");
    assert!(navigation.outgoing[3].targets.is_empty());
}

#[test]
fn ambiguous_pages_offer_only_real_heading_destinations_without_auto_disambiguation() {
    let model = WorkbenchReadModel::build(&[
        note("入口.md", "[[任务#本周]]"),
        note("甲/任务.md", "# 甲\n## 本周\n"),
        note("乙/任务.md", "# 乙\n## 其他\n"),
    ])
    .unwrap();
    let navigation = model.navigation(&path("入口.md")).unwrap();
    assert_eq!(navigation.outgoing[0].status, "ambiguous");
    assert_eq!(navigation.outgoing[0].targets.len(), 1);
    assert_eq!(navigation.outgoing[0].targets[0].path, "甲/任务.md");
    assert_eq!(navigation.outgoing[0].targets[0].line, 2);
}

#[test]
fn fenced_and_inline_code_and_escaped_links_do_not_create_navigation_items() {
    let body = "# 可见\n````md\n[[伪一]]\n~~~\n## 伪标题\n```\n[[伪二]]\n````\n`[[伪三]]` ``[[伪四]]`` \\[[伪五]] [[目标]]\n";
    let model =
        WorkbenchReadModel::build(&[note("入口.md", body), note("目标.md", "# 目标")]).unwrap();
    let navigation = model.navigation(&path("入口.md")).unwrap();
    assert_eq!(navigation.outline.len(), 1);
    assert_eq!(navigation.outgoing.len(), 1);
    assert_eq!(navigation.outgoing[0].source.line, 9);
    assert_eq!(navigation.outgoing[0].status, "resolved");
}

#[test]
fn all_navigation_lists_and_display_text_are_bounded_but_paths_are_complete() {
    let long_path = format!("项目/{}入口.md", "长目录/".repeat(90));
    let long_text = "长".repeat(400);
    let body = format!(
        "# {long_text}\n{}",
        format!("## 重复\n[[#重复|{long_text}]]\n").repeat(205)
    );
    let model = WorkbenchReadModel::build(&[note(&long_path, &body)]).unwrap();
    let navigation = model.navigation(&path(&long_path)).unwrap();
    assert!(navigation.truncated);
    assert_eq!(navigation.outline.len(), 200);
    assert_eq!(navigation.outgoing.len(), 200);
    assert_eq!(navigation.backlinks.len(), 200);
    let link = &navigation.outgoing[0];
    assert_eq!(link.status, "ambiguous_heading");
    assert_eq!(link.targets.len(), 50);
    assert_eq!(link.source.path, long_path);
    assert!(
        link.targets
            .iter()
            .all(|target| target.path == long_path && target.line > 1)
    );
    assert_eq!(link.label.chars().count(), 320);
    assert_eq!(link.raw.chars().count(), 320);
    assert!(
        navigation
            .outline
            .iter()
            .all(|entry| entry.title.chars().count() <= 320)
    );
    assert!(navigation.backlinks.iter().all(|entry| entry.title.chars().count() <= 320 && entry.context.chars().count() <= 320));
}

#[test]
fn page_candidates_are_bounded_and_navigation_is_owned_and_generation_local() {
    let mut inputs = vec![note("入口.md", "[[同名]]")];
    for number in 0..55 {
        inputs.push(note(&format!("{number:02}/同名.md"), "# 同名"));
    }
    let model = WorkbenchReadModel::build(&inputs).unwrap();
    let mut navigation = model.navigation(&path("入口.md")).unwrap();
    assert!(navigation.truncated);
    assert_eq!(navigation.outgoing[0].status, "ambiguous");
    assert_eq!(navigation.outgoing[0].targets.len(), 50);
    assert_eq!(navigation.outgoing[0].targets[0].path, "00/同名.md");
    navigation.outgoing.clear();
    assert_eq!(
        model.navigation(&path("入口.md")).unwrap().outgoing.len(),
        1
    );
    assert!(model.navigation(&path("未知.md")).is_none());
}
