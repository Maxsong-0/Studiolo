//! 只给计数的投影契约：计数有来源、不可得时显式说明，绝不把不可得退化成零。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::WorkbenchReadModel;

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).expect("测试用逻辑路径合法"), content)
}

fn model(documents: &[MarkdownDocument]) -> WorkbenchReadModel {
    WorkbenchReadModel::build(documents).expect("测试输入没有重复路径")
}

#[test]
fn every_count_sums_its_projects_and_matches_the_generation_it_came_from() {
    let documents = [
        note(
            "项目/甲/00-总览/INDEX.md",
            "---\nlifecycle: 活跃\n当前版本: v1\n---\n# 甲\n",
        ),
        note(
            "项目/甲/04-协调/TASKS.md",
            "## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-1 | 甲一 | 进行中 |\n| T-2 | 甲二 | 进行中 |\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 未修复\n---\n# 甲错\n",
        ),
        note(
            "项目/乙/04-协调/TASKS.md",
            "## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-9 | 乙一 | 进行中 |\n| T-8 | 乙旧 | 已完成 |\n",
        ),
        note(
            "项目/乙/06-BUG/BUG-9.md",
            "---\n状态: 已修复\n---\n# 乙错\n",
        ),
        note("笔记/随手.md", "# 与项目无关的一篇\n"),
    ];
    let model = model(&documents);
    let counts = model.counts();

    assert_eq!(counts.documents.value, Some(6));
    assert_eq!(counts.projects.value, Some(2));
    assert_eq!(counts.active_tasks.value, Some(3), "2 + 1，已完成的不计");
    assert_eq!(counts.open_bugs.value, Some(1), "未修复 1 + 已修复 0");
    for rollup in [
        &counts.documents,
        &counts.projects,
        &counts.active_tasks,
        &counts.open_bugs,
    ] {
        assert!(rollup.unavailable.is_empty(), "可得的计数没有不可得说明");
    }

    let snapshot = model.snapshot();
    assert_eq!(counts.inbox.value, Some(snapshot.inbox.len() as u64));
    assert_eq!(
        counts.diagnostics.value,
        Some(snapshot.diagnostics.len() as u64)
    );
    assert_eq!(model.document_count(), 6);
    assert_eq!(counts.documents.value, Some(model.document_count() as u64));
}

#[test]
fn a_project_whose_bug_status_cannot_be_classified_makes_the_rollup_unavailable_not_zero() {
    let documents = [
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 神秘状态\n---\n# 甲错\n",
        ),
        note(
            "项目/乙/06-BUG/BUG-9.md",
            "---\n状态: 未修复\n---\n# 乙错\n",
        ),
    ];
    let counts = model(&documents).counts();

    assert_eq!(counts.open_bugs.value, None, "不可得绝不退化成 0");
    assert_eq!(counts.open_bugs.unavailable, ["项目/甲"], "逐条点名来源");
    assert_eq!(counts.projects.value, Some(2), "汇总不可得不牵连基数");
}

#[test]
fn a_missing_task_table_makes_the_task_rollup_unavailable_and_names_every_such_project() {
    let documents = [
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 已修复\n---\n# 甲错\n",
        ),
        note(
            "项目/乙/04-协调/TASKS.md",
            "## ① 进行中\n| ID | 任务 |\n|---|---|\n| T-9 | 缺状态列不能宣称零任务 |\n",
        ),
        note(
            "项目/乙/06-BUG/BUG-9.md",
            "---\n状态: 已修复\n---\n# 乙错\n",
        ),
    ];
    let counts = model(&documents).counts();

    assert_eq!(counts.active_tasks.value, None);
    assert_eq!(
        counts.active_tasks.unavailable,
        ["项目/乙", "项目/甲"],
        "两个项目各自不可得，按项目标识的码位序逐条点名"
    );
    assert_eq!(counts.open_bugs.value, Some(0), "有证据的零仍然是零");
    assert!(counts.open_bugs.unavailable.is_empty());
}

#[test]
fn an_empty_generation_counts_zero_without_inventing_projects_or_rollups() {
    let counts = model(&[]).counts();

    assert_eq!(counts.documents.value, Some(0));
    assert_eq!(counts.projects.value, Some(0));
    assert_eq!(counts.inbox.value, Some(0));
    assert_eq!(counts.diagnostics.value, Some(0));
    assert_eq!(
        counts.active_tasks.value,
        Some(0),
        "没有项目就没有不可得来源"
    );
    assert_eq!(counts.open_bugs.value, Some(0));
    assert!(counts.active_tasks.unavailable.is_empty());
}

#[test]
fn the_overview_carries_the_same_projects_inbox_and_diagnostics_as_the_snapshot() {
    let documents = [
        note(
            "项目/甲/00-总览/INDEX.md",
            "---\nlifecycle: 活跃\n当前版本: v1\n---\n# 甲\n- **用户侧待办**：验收计数投影\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 神秘状态\n---\n# 甲错\n",
        ),
        note("笔记/随手.md", "# 与项目无关的一篇\n"),
    ];
    let model = model(&documents);
    let overview = model.overview();
    let snapshot = model.snapshot();

    assert_eq!(overview.projects, snapshot.projects);
    assert_eq!(overview.inbox, snapshot.inbox);
    assert_eq!(overview.diagnostics, snapshot.diagnostics);
    assert_eq!(overview.counts, model.counts());
    assert_eq!(
        overview.counts.documents.value,
        Some(snapshot.documents.len() as u64),
        "首屏不带逐篇清单，但文档数仍然如实给出"
    );
    assert!(!overview.inbox.is_empty(), "首屏仍然给出待我动作");
}
