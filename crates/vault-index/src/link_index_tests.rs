//! 索引内部不变量：增量维护与全量重建等价，且单条解析的访问量与库规模无关。

use std::collections::BTreeMap;

use super::*;
use crate::{
    LinkResolution, MarkdownDocument, ResolutionRule, WikilinkTarget,
    link_resolve::{ResolveProbe, resolve_probed},
    parse::ParsedDocument,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn document(path_text: &str, body: &str) -> IndexedDocument {
    let input = MarkdownDocument::new(path(path_text), body);
    ParsedDocument::parse(&input).finish(input.path(), None)
}

fn corpus(entries: &[(&str, &str)]) -> BTreeMap<VaultPath, IndexedDocument> {
    entries
        .iter()
        .map(|(path_text, body)| {
            let document = document(path_text, body);
            (document.path.clone(), document)
        })
        .collect()
}

fn generated(total: usize) -> BTreeMap<VaultPath, IndexedDocument> {
    (0..total)
        .map(|number| {
            let document = document(
                &format!("项目/{}/笔记/N{number:06}.md", number % 16),
                &format!("# N{number:06}\n"),
            );
            (document.path.clone(), document)
        })
        .collect()
}

#[test]
fn incremental_apply_produces_exactly_the_same_index_as_a_full_rebuild() {
    let before = corpus(&[
        ("项目/甲/日志/今天.md", "# 今天"),
        (
            "项目/甲/协调/TASKS.md",
            "---\naliases: [协调中心]\n---\n# 甲任务",
        ),
        ("项目/乙/协调/TASKS.md", "# 乙任务"),
    ]);
    let after = corpus(&[
        // 标题与 alias 都变了：必须重索引这一篇。
        (
            "项目/甲/协调/TASKS.md",
            "---\naliases: [新中心]\n---\n# 改名",
        ),
        // 新增一篇，删除「今天.md」。
        ("项目/乙/协调/TASKS.md", "# 乙任务"),
        ("项目/丙/笔记/新页.md", "# 新页"),
    ]);

    let mut incremental = LinkIndex::rebuilt(&before);
    let update = incremental.apply(&after);

    assert_eq!(update.removed, 1, "「今天.md」应当退出候选集合");
    assert_eq!(update.inserted, 2, "改名的一篇与新增的一篇需要重索引");
    assert_eq!(update.unchanged, 1, "未变文档不应被重复索引");
    assert_eq!(
        incremental,
        LinkIndex::rebuilt(&after),
        "增量维护结果必须与从文件完全重建一致"
    );
    assert!(
        incremental.names_for("协调中心").is_empty(),
        "旧 alias 必须随重索引一并撤下"
    );
    assert_eq!(incremental.names_for("新中心").len(), 1);
}

#[test]
fn applying_the_same_documents_twice_touches_nothing() {
    let documents = generated(64);
    let mut index = LinkIndex::rebuilt(&documents);
    let update = index.apply(&documents);

    assert_eq!(update.inserted, 0);
    assert_eq!(update.removed, 0);
    assert_eq!(update.unchanged, documents.len());
    assert_eq!(index, LinkIndex::rebuilt(&documents));
}

#[test]
fn an_emptied_index_rebuilds_completely_from_the_current_documents() {
    let documents = corpus(&[
        ("项目/甲/协调/TASKS.md", "# 甲任务"),
        ("项目/乙/协调/别名.md", "---\naliases: [索引]\n---\n# 乙"),
    ]);
    let mut lost = LinkIndex::default();

    lost.apply(&documents);

    assert_eq!(lost, LinkIndex::rebuilt(&documents));
    assert_eq!(lost.names_for("索引").len(), 1);
}

#[test]
fn one_resolution_examines_the_same_candidate_count_no_matter_how_large_the_vault_is() {
    let from = path("项目/1/笔记/N000001.md");
    // 三条解析路径各取一例：就近同名、Vault 根精确路径、零候选悬空。
    let cases = [
        (
            "[[N000009]]",
            LinkResolution::Resolved {
                path: path("项目/9/笔记/N000009.md"),
                rule: ResolutionRule::NearestUnique { ancestor_depth: 2 },
            },
        ),
        (
            "[[项目/9/笔记/N000009]]",
            LinkResolution::Resolved {
                path: path("项目/9/笔记/N000009.md"),
                rule: ResolutionRule::VaultRootPath,
            },
        ),
        ("[[从来没有这一篇]]", LinkResolution::Dangling),
    ];

    // 规模证据：同一条链接在三种库规模下的候选访问量必须逐例相等。
    let sizes = [500_usize, 4000, 16_000];
    let mut measured: Vec<Vec<(usize, ResolveProbe)>> = Vec::new();
    for total in sizes {
        let documents = generated(total);
        let index = LinkIndex::rebuilt(&documents);
        let mut row = Vec::new();
        for (raw, expected) in &cases {
            let target = WikilinkTarget::parse(raw).expect("valid wikilink");
            let (resolution, probe) = resolve_probed(&index, &from, &target);
            assert_eq!(&resolution, expected, "{raw} 在 {total} 篇库上解析结果变了");
            row.push((total, probe));
        }
        measured.push(row);
    }

    for (case, (raw, _)) in cases.iter().enumerate() {
        let column: Vec<_> = measured
            .iter()
            .filter_map(|row| row.get(case).copied())
            .collect();
        assert_eq!(column.len(), sizes.len());
        assert!(
            column.windows(2).all(|pair| pair[0].1.candidates_examined
                == pair[1].1.candidates_examined
                && pair[0].1.ancestor_levels == pair[1].1.ancestor_levels),
            "{raw} 的候选访问量必须与库规模无关，实测 {column:?}"
        );
        assert!(
            column
                .first()
                .is_some_and(|(_, probe)| probe.candidates_examined <= 4),
            "{raw} 单次解析只应访问同名或同路径候选，实测 {column:?}"
        );
    }
}
