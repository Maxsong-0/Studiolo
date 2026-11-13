//! 经公开接口验证限定范围的搜索：前缀与逐路径许可都在计数之前生效，
//! 100 条上限与 `truncated` 只数范围内、许可内的命中（BUG-036）。

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{
    SearchError, SearchHit, SearchHitKind, SearchMode, SearchQuery, SearchResults, SearchScope,
    WorkbenchReadModel,
};

const WORD: &str = "探针";
/// 恰好一行含 [`WORD`]；路径里不含它，所以全文搜索每篇只命中一条。
const BODY: &str = "含探针的一行\n";
const ALPHA: &str = "项目/Alpha/";
const BETA: &str = "项目/Beta/";

fn note(path: &str, content: impl Into<String>) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

/// 前缀下 `count` 篇笔记，按编号字典序排列。
fn crowd(prefix: &str, count: usize) -> Vec<MarkdownDocument> {
    (0..count)
        .map(|index| note(&format!("{prefix}笔记-{index:03}.md"), BODY))
        .collect()
}

fn beta(index: usize) -> String {
    format!("{BETA}笔记-{index:03}.md")
}

/// 前面的项目有 `crowded` 篇命中，后面的项目有 `later` 篇。
fn two_projects(crowded: usize, later: usize) -> WorkbenchReadModel {
    let mut documents = crowd(ALPHA, crowded);
    documents.extend(crowd(BETA, later));
    WorkbenchReadModel::build(&documents).unwrap()
}

fn query(text: &str, mode: SearchMode) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        project: None,
        mode,
    }
}

fn within(
    model: &WorkbenchReadModel,
    request: &SearchQuery,
    prefix: &str,
    permitted: &dyn Fn(&str) -> bool,
) -> Result<SearchResults, SearchError> {
    model.search_within(request, &SearchScope { prefix, permitted })
}

fn paths(found: &SearchResults) -> Vec<&str> {
    found.hits.iter().map(|hit| hit.path.as_str()).collect()
}

fn all_under(found: &SearchResults, prefix: &str) -> bool {
    found.hits.iter().all(|hit| hit.path.starts_with(prefix))
}

fn is_path_hit(hit: &SearchHit) -> bool {
    hit.kind == SearchHitKind::Path
}

#[test]
fn a_later_project_keeps_its_hits_when_an_earlier_project_has_more_than_the_limit() {
    let model = two_projects(150, 3);
    let request = query(WORD, SearchMode::FullText);

    // 全库搜索的名额被前面的项目占满：这正是限定范围必须先于计数的原因。
    let global = model.search(&request).unwrap();
    assert_eq!(global.hits.len(), 100);
    assert!(global.truncated);
    assert!(all_under(&global, ALPHA));

    let scoped = within(&model, &request, BETA, &|_| true).unwrap();
    assert_eq!(paths(&scoped), [beta(0), beta(1), beta(2)]);
    assert!(!scoped.truncated);
}

#[test]
fn a_document_the_caller_does_not_permit_is_never_a_hit() {
    let model = two_projects(150, 3);
    let request = query(WORD, SearchMode::FullText);
    let rejected = beta(1);
    let permitted = |path: &str| path != rejected;
    let scoped = within(&model, &request, BETA, &permitted).unwrap();
    assert_eq!(paths(&scoped), [beta(0), beta(2)]);
    assert!(!scoped.truncated);
}

#[test]
fn the_limit_and_truncation_count_only_permitted_hits_inside_the_scope() {
    let model = two_projects(150, 101);
    let request = query(WORD, SearchMode::FullText);

    // 拒掉一篇后范围内恰好 100 条：不算截断。
    let rejected = beta(50);
    let permitted = |path: &str| path != rejected;
    let exact = within(&model, &request, BETA, &permitted).unwrap();
    assert_eq!(exact.hits.len(), 100);
    assert!(!exact.truncated);
    assert!(!paths(&exact).contains(&rejected.as_str()));
    assert!(all_under(&exact, BETA));

    // 全部许可时观察到第 101 条：截到 100 条并标出截断。
    let full = within(&model, &request, BETA, &|_| true).unwrap();
    assert_eq!(full.hits.len(), 100);
    assert!(full.truncated);
    assert_eq!(full.hits[99].path, beta(99));
}

#[test]
fn malformed_scopes_or_a_second_project_filter_are_rejected() {
    let model = two_projects(0, 3);
    let request = query(WORD, SearchMode::FullText);
    for prefix in ["", "项目/Beta"] {
        let rejected = within(&model, &request, prefix, &|_| true);
        assert_eq!(rejected, Err(SearchError::InvalidProject), "{prefix:?}");
    }

    let mut with_project = request.clone();
    with_project.project = Some("项目/Beta".into());
    let rejected = within(&model, &with_project, BETA, &|_| true);
    assert_eq!(rejected, Err(SearchError::InvalidProject));

    // 查询文本的约束与全库搜索一致：范围合法也照样先拒。
    let control = query("a\nb", SearchMode::FullText);
    let rejected = within(&model, &control, BETA, &|_| true);
    assert_eq!(rejected, Err(SearchError::InvalidQuery));
}

#[test]
fn an_adjacent_project_name_does_not_leak_into_the_scope() {
    let documents = [
        note("项目/乙/甲.md", BODY),
        note("项目/乙丙/乙.md", BODY),
        note("项目/乙-旁/丙.md", BODY),
    ];
    let model = WorkbenchReadModel::build(&documents).unwrap();
    let prefix = "项目/乙/";
    for (mode, text) in [(SearchMode::FullText, WORD), (SearchMode::Path, "")] {
        let scoped = within(&model, &query(text, mode), prefix, &|_| true).unwrap();
        assert_eq!(paths(&scoped), ["项目/乙/甲.md"], "{mode:?}");
    }
}

#[test]
fn path_mode_obeys_the_same_scope_permission_and_limit() {
    let request = query("笔记", SearchMode::Path);

    let model = two_projects(150, 3);
    let scoped = within(&model, &request, BETA, &|_| true).unwrap();
    assert_eq!(paths(&scoped), [beta(0), beta(1), beta(2)]);
    assert!(!scoped.truncated);
    assert!(scoped.hits.iter().all(is_path_hit));

    let rejected = beta(0);
    let permitted = |path: &str| path != rejected;
    let scoped = within(&model, &request, BETA, &permitted).unwrap();
    assert_eq!(paths(&scoped), [beta(1), beta(2)]);

    let model = two_projects(150, 101);
    let rejected = beta(100);
    let permitted = |path: &str| path != rejected;
    let exact = within(&model, &request, BETA, &permitted).unwrap();
    assert_eq!(exact.hits.len(), 100);
    assert!(!exact.truncated);
    let full = within(&model, &request, BETA, &|_| true).unwrap();
    assert_eq!(full.hits.len(), 100);
    assert!(full.truncated);
}
