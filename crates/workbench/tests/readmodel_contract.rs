//! M1 读模型的公开接口契约：纯派生，且各项都链接到源位置。

use studiolo_core::VaultPath;
use studiolo_vault_index::{IndexError, MarkdownDocument};
use studiolo_workbench::WorkbenchReadModel;

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

#[test]
fn tables_do_not_bridge_code_fences_or_count_repeated_headers() {
    let docs = [note(
        "项目/甲/04-协调/TASKS.md",
        "## ① 进行中\n| ID | 任务 | 状态 |\n```md\n代码示例\n```\n|---|---|---|\n| T-fake | 不是原文中完整的表 | 待用户验收 |\n\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-1 | 真正的表格 | 待用户验收 |\n| ID | 任务 | 状态 |\n|---|---|---|\n",
    )];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    assert_eq!(snapshot.projects[0].active_tasks.value, Some(1));
    assert_eq!(snapshot.inbox.len(), 1);
    assert_eq!(snapshot.inbox[0].source.line, 11);
}

#[test]
fn snapshot_order_and_diagnostics_do_not_depend_on_input_order() {
    let first = note(
        "项目/乙/原文.md",
        "---\nstatus: done\n状态: 已完成\n---\n# 乙\n",
    );
    let second = note(
        "项目/甲/原文.md",
        "---\nstatus: done\n状态: 已完成\n---\n# 甲\n",
    );
    let forward = WorkbenchReadModel::build(&[first.clone(), second.clone()])
        .unwrap()
        .snapshot();
    let backward = WorkbenchReadModel::build(&[second, first])
        .unwrap()
        .snapshot();
    assert_eq!(forward, backward);
    assert_eq!(forward.diagnostics.len(), 2);
}

#[test]
fn unfiled_projects_are_visible_without_fabricated_defaults() {
    let model = WorkbenchReadModel::build(&[
        note("项目/灵感盒/功能.md", "# 功能\n等待用户确认只是普通正文"),
        note(
            "projects/not-a-project/README.md",
            "# ignored project prefix",
        ),
        note("日记/今天.md", "# 今天"),
    ])
    .unwrap();
    let snapshot = model.snapshot();
    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.name, "灵感盒");
    assert_eq!(project.profile, "unfiled");
    assert_eq!(project.lifecycle.value, None);
    assert_eq!(project.version.value, None);
    assert_eq!(project.active_tasks.value, None);
    assert_eq!(project.open_bugs.value, None);
    assert_eq!(project.next_milestone.value, None);
    assert!(project.source.is_none());
    assert!(!project.diagnostics.is_empty());
    assert!(snapshot.inbox.is_empty());
    assert_eq!(snapshot.documents.len(), 3);
}

#[test]
fn chinese_fields_have_exact_sources_and_current_sections_only() {
    let docs = [
        note(
            "项目/纸页/00-总览/INDEX.md",
            "# 纸页\n\n| 项 | 当前值 |\n|---|---|\n| 生命周期 | 活跃 |\n| 当前版本 | v0.3 |\n\n## 用户侧待办\n- [ ] 确认阅读页布局\n- [x] 已完成的验收\n\n## 历史\n- 等待用户重启旧服务\n",
        ),
        note(
            "项目/纸页/04-协调/TASKS.md",
            "# TASKS\n## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-1 | 阅读页 | 进行中 |\n| T-2 | 原文定位 | 阻塞，待用户验收 |\n| T-3 | 旧任务 | 已完成 |\n| | | |\n\n## ② 占用与编号登记\n| 资源 | 状态 |\n|---|---|\n| 端口 | 待用户重启 |\n\n## ③ 待重启\n| 改动内容 | 需要的操作（重启哪个服务） | 登记线 | 日期 |\n|---|---|---|---|\n| 应用本机配置变更 | 重启纸页 | Codex | 2026-09-04 |\n| | | | |\n\n## ④ 已完成\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-9 | 等待用户验收旧版 | 已完成 |\n",
        ),
        note(
            "项目/纸页/05-版本/roadmap.md",
            "# Roadmap\n| 里程碑 | 状态 |\n|---|---|\n| M0 | 已完成 |\n| M1 | 规划中 |\n| M2 | 未完成 |\n",
        ),
        note(
            "项目/纸页/06-BUG/BUG-001.md",
            "---\n状态: 未修复\n---\n# 中文 BUG\n",
        ),
        note(
            "项目/纸页/02-日志/今天.md",
            "## 用户侧待办\n- 等待用户重启服务\n",
        ),
        note(
            "项目/纸页/04-协调/归档/TASKS.md",
            "## ③ 待重启\n| 改动内容 | 操作 |\n|---|---|\n| 老配置 | 重启 |",
        ),
    ];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    let project = &snapshot.projects[0];
    assert_eq!(project.profile, "standard");
    assert_eq!(project.lifecycle.value.as_deref(), Some("活跃"));
    assert_eq!(project.lifecycle.source.as_ref().unwrap().line, 5);
    assert_eq!(project.version.value.as_deref(), Some("v0.3"));
    assert_eq!(project.version.source.as_ref().unwrap().line, 6);
    assert_eq!(project.active_tasks.value, Some(2));
    assert_eq!(project.open_bugs.value, Some(1));
    assert_eq!(project.open_bugs.source.as_ref().unwrap().line, 2);
    assert_eq!(project.next_milestone.value.as_deref(), Some("M1"));
    assert_eq!(project.next_milestone.source.as_ref().unwrap().line, 5);
    assert_eq!(snapshot.inbox.len(), 3);
    assert!(
        snapshot
            .inbox
            .iter()
            .any(|item| item.kind == "待拍板" && item.source.line == 9)
    );
    assert!(
        snapshot
            .inbox
            .iter()
            .any(|item| item.kind == "待验收" && item.source.line == 6)
    );
    assert!(
        snapshot
            .inbox
            .iter()
            .any(|item| item.kind == "待重启" && item.source.line == 18)
    );
    assert!(
        snapshot
            .inbox
            .iter()
            .all(|item| !item.source.path.contains("归档") && !item.source.path.contains("日志"))
    );
}

#[test]
fn empty_tables_code_fences_and_undeclared_lifecycle_are_not_guessed() {
    let docs = [
        note(
            "项目/山野/00-总览/INDEX.md",
            "# 山野\n| 项 | 当前值 |\n|---|---|\n| 状态 | 活跃，等待用户验收 |\n\n````md\n## 用户侧待办\n- [ ] 重启\n```\n- [ ] 仍在围栏中\n````\n",
        ),
        note(
            "项目/山野/04-协调/TASKS.md",
            "# TASKS\n## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| | | |\n| T-XXX | 待补充 | 进行中 |\n\n## ③ 待重启\n| 改动内容 | 操作 |\n|---|---|\n| | |\n",
        ),
        note(
            "项目/山野/05-版本/roadmap.md",
            "# Roadmap\n| 里程碑 | 状态 |\n|---|---|\n| M1 | 未完成 |\n",
        ),
    ];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    assert_eq!(snapshot.projects[0].lifecycle.value, None);
    assert_eq!(snapshot.projects[0].active_tasks.value, Some(0));
    assert!(snapshot.projects[0].active_tasks.source.is_some());
    assert_eq!(
        snapshot.projects[0].next_milestone.value.as_deref(),
        Some("M1")
    );
    assert!(snapshot.inbox.is_empty());
}

#[test]
fn ambiguous_bug_status_remains_unknown_and_only_explicit_frontmatter_creates_actions() {
    let docs = [
        note(
            "项目/甲/00-总览/INDEX.md",
            "---\nlifecycle: 暂停\n当前版本: v1\n---\n# 甲\n- **用户侧待办**：验收本机只读工作台\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\nstatus: 已修复\n---\n# 修好的 BUG\n等待用户验收\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-2.md",
            "---\n状态: 神秘状态\n---\n# 未知 BUG\n",
        ),
        note(
            "项目/甲/03-记忆/ADR-001.md",
            "---\n状态: 待用户拍板\n---\n# 技术选择\n",
        ),
        note(
            "项目/甲/03-记忆/归档/ADR-000.md",
            "---\n状态: 待用户重启\n---\n# 旧决策\n",
        ),
        note(
            "项目/甲/03-记忆/领域知识.md",
            "---\n状态: 待用户拍板\n---\n# 不是 ADR\n",
        ),
    ];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    assert_eq!(
        snapshot.projects[0].lifecycle.value.as_deref(),
        Some("暂停")
    );
    assert_eq!(
        snapshot.projects[0].lifecycle.source.as_ref().unwrap().line,
        2
    );
    assert_eq!(snapshot.projects[0].open_bugs.value, None);
    assert!(
        snapshot.projects[0]
            .diagnostics
            .iter()
            .any(|notice| notice.contains("BUG-2"))
    );
    assert_eq!(snapshot.inbox.len(), 2);
    assert!(
        snapshot
            .inbox
            .iter()
            .any(|item| item.kind == "待拍板" && item.source.line == 2)
    );
    assert!(
        snapshot
            .inbox
            .iter()
            .any(|item| item.kind == "待验收" && item.source.line == 6)
    );
}

#[test]
fn document_reads_preserve_original_content_and_duplicate_paths_reject_the_build() {
    let text = "---\ntitle: 标题\n---\r\n# 原文\r\n[[别页|名称]]";
    let input = note("项目/甲/原文.md", text);
    let model = WorkbenchReadModel::build(std::slice::from_ref(&input)).unwrap();
    let mut copy = model.document(input.path()).unwrap();
    assert_eq!(copy.content, text);
    assert_eq!(copy.title, "标题");
    copy.content.clear();
    assert_eq!(model.document(input.path()).unwrap().content, text);
    assert!(
        model
            .document(&VaultPath::parse("项目/甲/不存在.md").unwrap())
            .is_none()
    );
    assert!(matches!(
        WorkbenchReadModel::build(&[input.clone(), input]),
        Err(IndexError::DuplicatePath(_))
    ));
}

#[test]
fn controlled_waiting_states_and_parenthesized_user_actions_are_current_actions() {
    let docs = [note(
        "项目/甲/04-协调/TASKS.md",
        "## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-1 | 阅读页 | 待验收 |\n| T-2 | 来源设计 | 待拍板 |\n| T-3 | 基线 | 阻塞（待用户确认基线） |\n| T-4 | 等待用户确认只是旧标题 | 进行中 |\n| T-5 | 无需用户操作 | 进行中（无需用户确认） |\n\n## ③ 待重启\n| 改动内容 | 需要的操作 | 状态 |\n|---|---|---|\n| 旧变更 | 重启服务 | 已完成 |\n| [x] 更旧变更 | 重启服务 | |\n| 配置已生效 | 已重启 | |\n",
    )];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    assert_eq!(snapshot.projects[0].active_tasks.value, Some(5));
    assert_eq!(snapshot.inbox.len(), 3);
    assert_eq!(snapshot.inbox[0].kind, "待验收");
    assert_eq!(snapshot.inbox[1].kind, "待拍板");
    assert_eq!(snapshot.inbox[2].kind, "待拍板");
    assert!(snapshot.inbox.iter().all(|item| item.source.line <= 6));
}

#[test]
fn missing_task_schema_is_unknown_and_closed_bugs_supply_evidence_for_zero() {
    let docs = [
        note(
            "项目/甲/04-协调/TASKS.md",
            "## ① 进行中\n| ID | 任务 |\n|---|---|\n| T-1 | 不能凭缺失状态宣称零任务 |\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 已修复\n---\n# Fixed\n",
        ),
        note(
            "项目/甲/06-BUG/归档/BUG-old.md",
            "---\n状态: 未修复\n---\n# Old\n",
        ),
    ];
    let snapshot = WorkbenchReadModel::build(&docs).unwrap().snapshot();
    assert_eq!(snapshot.projects[0].active_tasks.value, None);
    assert_eq!(snapshot.projects[0].open_bugs.value, Some(0));
    assert_eq!(
        snapshot.projects[0].open_bugs.source.as_ref().unwrap().path,
        "项目/甲/06-BUG/BUG-1.md"
    );
    assert_eq!(
        snapshot.projects[0].open_bugs.source.as_ref().unwrap().line,
        2
    );
}
