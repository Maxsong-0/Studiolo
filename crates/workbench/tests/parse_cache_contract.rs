//! 持久解析只改善构建；每轮正文和项目状态仍来自新输入。

use studiolo_core::VaultPath;
use studiolo_vault_index::{MarkdownDocument, ParseCache, ParseCacheStats};
use studiolo_workbench::{SearchMode, SearchQuery, WorkbenchReadModel};

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

#[test]
fn warm_workbench_matches_all_uncached_views_and_search() {
    let documents = vec![
        note(
            "项目/甲/00-总览/INDEX.md",
            "---\nlifecycle: 活跃\ncurrent_version: M3\n---\n# 甲\n[[目标]]\n",
        ),
        note("项目/甲/目标.md", "# 目标\n只存在输入的正文标记\n## 小节\n"),
        note(
            "项目/甲/04-协调/TASKS.md",
            "# TASKS\n## ① 进行中\n| 任务 | 状态 |\n| --- | --- |\n| 当前任务 | 进行中 |\n",
        ),
        note(
            "项目/甲/05-版本/roadmap.md",
            "# 路线\n| 里程碑 | 状态 |\n| --- | --- |\n| M3 | 进行中 |\n",
        ),
    ];
    let ordinary = WorkbenchReadModel::build(&documents).unwrap();
    let (_, cache, _) = WorkbenchReadModel::build_cached(&documents, None).unwrap();
    let bytes = cache.encode().unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("只存在输入的正文标记")
    );
    let decoded = ParseCache::decode(&bytes).unwrap();
    let (warm, _, stats) = WorkbenchReadModel::build_cached(&documents, Some(&decoded)).unwrap();
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 4,
            parsed: 0
        }
    );
    assert_eq!(ordinary.snapshot(), warm.snapshot());
    for input in &documents {
        assert_eq!(ordinary.document(input.path()), warm.document(input.path()));
        assert_eq!(
            ordinary.navigation(input.path()),
            warm.navigation(input.path())
        );
    }
    for (text, mode) in [
        ("正文标记", SearchMode::FullText),
        ("甲 目标", SearchMode::Path),
    ] {
        let query = SearchQuery {
            text: text.into(),
            project: Some("项目/甲".into()),
            mode,
        };
        assert_eq!(ordinary.search(&query), warm.search(&query));
    }
}

#[test]
fn changed_source_body_and_project_status_replace_previous_derived_values() {
    let path = "项目/甲/00-总览/INDEX.md";
    let old = [note(path, "---\ncurrent_version: M2\n---\n# 项目\n旧正文")];
    let (_, cache, _) = WorkbenchReadModel::build_cached(&old, None).unwrap();
    let current = [note(path, "---\ncurrent_version: M3\n---\n# 项目\n新正文")];
    let (model, _, stats) = WorkbenchReadModel::build_cached(&current, Some(&cache)).unwrap();
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 0,
            parsed: 1
        }
    );
    assert_eq!(
        model.document(current[0].path()).unwrap().content,
        current[0].contents()
    );
    assert_eq!(
        model.snapshot().projects[0].version.value.as_deref(),
        Some("M3")
    );
    let results = model
        .search(&SearchQuery {
            text: "旧正文".into(),
            project: None,
            mode: SearchMode::FullText,
        })
        .unwrap();
    assert!(results.hits.is_empty());
}

#[test]
fn absent_input_cannot_be_resurrected_by_a_previous_cache() {
    let old = [note("项目/甲/已移除.md", "# 已移除\n秘密旧正文")];
    let (_, cache, _) = WorkbenchReadModel::build_cached(&old, None).unwrap();
    let (empty, _, stats) = WorkbenchReadModel::build_cached(&[], Some(&cache)).unwrap();
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 0,
            parsed: 0
        }
    );
    assert!(empty.snapshot().documents.is_empty());
    assert!(empty.snapshot().projects.is_empty());
    assert!(empty.document(old[0].path()).is_none());
}
