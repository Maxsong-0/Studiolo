//! 有界性证据：首屏投影保留的字节数不随本代文档数增长，逐篇清单才随之线性增长。
//!
//! 这里量的是投影**实际持有的字符串字节数**，逐字段走遍返回值累加，而不是计时，
//! 也不是「看起来应该是常数」的断言。工作区禁止 unsafe，本 crate 也禁止进程与
//! 文件系统访问，所以进程级峰值 RSS 不在这里度量；本文件只声称它真正量到的东西。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{
    ActionItem, ProjectSummary, SourceLocation, Sourced, WorkbenchCounts, WorkbenchOverview,
    WorkbenchReadModel, WorkbenchSnapshot,
};

const SMALL: usize = 100;
const LARGE: usize = 2_000;

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).expect("测试用逻辑路径合法"), content)
}

/// 一个项目加上 `notes` 篇与项目无关的笔记；项目部分在两种规模下完全相同。
fn vault(notes: usize) -> Vec<MarkdownDocument> {
    let mut documents = vec![
        note(
            "项目/甲/00-总览/INDEX.md",
            "---\nlifecycle: 活跃\n当前版本: v1\n---\n# 甲\n- **用户侧待办**：验收首屏投影\n",
        ),
        note(
            "项目/甲/04-协调/TASKS.md",
            "## ① 进行中\n| ID | 任务 | 状态 |\n|---|---|---|\n| T-1 | 首屏投影 | 进行中 |\n",
        ),
        note(
            "项目/甲/06-BUG/BUG-1.md",
            "---\n状态: 未修复\n---\n# 甲错\n",
        ),
    ];
    for index in 0..notes {
        documents.push(note(
            &format!("笔记/{index:05}.md"),
            &format!("# 第 {index} 篇\n正文只影响逐篇清单之外的内存，不影响首屏投影。\n"),
        ));
    }
    documents
}

fn location_bytes(source: Option<&SourceLocation>) -> usize {
    source.map_or(0, |source| source.path.len())
}

fn sourced_text_bytes(value: &Sourced<String>) -> usize {
    value.value.as_ref().map_or(0, String::len) + location_bytes(value.source.as_ref())
}

fn project_bytes(project: &ProjectSummary) -> usize {
    project.id.len()
        + project.name.len()
        + project.root.len()
        + project.profile.len()
        + sourced_text_bytes(&project.lifecycle)
        + sourced_text_bytes(&project.version)
        + sourced_text_bytes(&project.next_milestone)
        + location_bytes(project.active_tasks.source.as_ref())
        + location_bytes(project.open_bugs.source.as_ref())
        + location_bytes(project.source.as_ref())
        + project.diagnostics.iter().map(String::len).sum::<usize>()
}

fn action_bytes(action: &ActionItem) -> usize {
    action.id.len()
        + action.project_id.len()
        + action.project_name.len()
        + action.kind.len()
        + action.title.len()
        + action.source.path.len()
}

fn counts_bytes(counts: &WorkbenchCounts) -> usize {
    [
        &counts.documents,
        &counts.projects,
        &counts.inbox,
        &counts.diagnostics,
        &counts.active_tasks,
        &counts.open_bugs,
    ]
    .into_iter()
    .map(|rollup| rollup.unavailable.iter().map(String::len).sum::<usize>())
    .sum()
}

fn overview_bytes(overview: &WorkbenchOverview) -> usize {
    overview.projects.iter().map(project_bytes).sum::<usize>()
        + overview.inbox.iter().map(action_bytes).sum::<usize>()
        + overview.diagnostics.iter().map(String::len).sum::<usize>()
        + counts_bytes(&overview.counts)
}

fn snapshot_bytes(snapshot: &WorkbenchSnapshot) -> usize {
    snapshot.projects.iter().map(project_bytes).sum::<usize>()
        + snapshot.inbox.iter().map(action_bytes).sum::<usize>()
        + snapshot.diagnostics.iter().map(String::len).sum::<usize>()
        + snapshot
            .documents
            .iter()
            .map(|document| document.path.len() + document.title.len())
            .sum::<usize>()
}

fn model(notes: usize) -> WorkbenchReadModel {
    WorkbenchReadModel::build(&vault(notes)).expect("测试输入没有重复路径")
}

#[test]
fn the_overview_holds_exactly_the_same_bytes_at_twenty_times_the_library_size() {
    let small = model(SMALL);
    let large = model(LARGE);
    let small_bytes = overview_bytes(&small.overview());
    let large_bytes = overview_bytes(&large.overview());

    assert_eq!(
        small_bytes, large_bytes,
        "首屏投影保留的字节数与本代文档数无关"
    );
    assert!(small_bytes > 0, "度量本身确实量到了东西");
    assert_eq!(small.document_count(), SMALL + 3);
    assert_eq!(large.document_count(), LARGE + 3);
    assert_eq!(
        large.overview().counts.documents.value,
        Some(large.document_count() as u64),
        "不带逐篇清单不等于数不出文档数"
    );
}

#[test]
fn the_per_document_list_is_what_grows_linearly_with_the_library() {
    let small = model(SMALL).snapshot();
    let large = model(LARGE).snapshot();
    let growth = snapshot_bytes(&large) - snapshot_bytes(&small);
    let added = LARGE - SMALL;

    assert_eq!(large.documents.len(), LARGE + 3);
    assert!(
        growth >= added * 20,
        "逐篇清单随文档数线性增长：{added} 篇多出 {growth} 字节"
    );
    assert!(
        snapshot_bytes(&large) > overview_bytes(&model(LARGE).overview()) * 50,
        "同一代次里逐篇清单比首屏投影大出一个数量级以上"
    );
}

#[test]
fn a_bigger_library_does_not_change_one_projects_derived_facts() {
    let small = model(SMALL).overview();
    let large = model(LARGE).overview();

    assert_eq!(
        small.projects, large.projects,
        "按前缀取区间，不受无关笔记影响"
    );
    assert_eq!(small.inbox, large.inbox);
    assert_eq!(small.diagnostics, large.diagnostics);
    assert_eq!(small.counts.active_tasks, large.counts.active_tasks);
    assert_eq!(small.counts.open_bugs, large.counts.open_bugs);
    assert_ne!(
        small.counts.documents, large.counts.documents,
        "只有文档数这一项应当随规模变化"
    );
}
