//! wikilink 解析索引的公开契约之二：进索引的键与查索引的键必须是同一套。
//!
//! 别名、锚点、大小写折叠与嵌套列表都要命中；代码块与转义不得贡献候选；NFC 拼写是路径侧
//! 的既有前提；解析结论不随库规模变化。

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, LinkResolution, MarkdownDocument, ResolutionRule, VaultIndex,
    WikilinkTarget,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn note(path_text: &str, contents: &str) -> MarkdownDocument {
    MarkdownDocument::new(path(path_text), contents)
}

fn link(raw: &str) -> WikilinkTarget {
    WikilinkTarget::parse(raw).expect("valid wikilink")
}

#[test]
fn aliases_anchors_case_folding_and_nested_lists_all_resolve_through_the_same_index() {
    let journal = path("项目/甲/02-日志/今天.md");
    let tasks = path("项目/甲/04-协调/TASKS.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(
                journal.as_str(),
                "- 上级\n\t- [[协调中心]]\n\t- [[tasks]]\n\t- [[TASKS#本周|本周板]]\n\t- [[#当天小结]]\n\n## 当天小结",
            ),
            note(
                tasks.as_str(),
                "---\naliases: [协调中心]\n---\n# 甲任务\n## 本周",
            ),
        ])
        .expect("rebuild succeeds");

    // 别名命中。
    assert_eq!(
        index
            .resolve_link(&journal, &link("[[协调中心]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: tasks.clone(),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );
    // 大小写折叠：`tasks` 与 `TASKS` 同一个键。
    assert_eq!(
        index
            .resolve_link(&journal, &link("[[tasks]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: tasks.clone(),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );
    // 锚点只作为片段保留，不参与候选查找。
    let anchored = link("[[TASKS#本周|本周板]]");
    assert_eq!(anchored.heading.as_deref(), Some("本周"));
    assert_eq!(anchored.alias.as_deref(), Some("本周板"));
    assert_eq!(
        index
            .resolve_link(&journal, &anchored)
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: tasks.clone(),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );
    // 纯锚点链接指向本篇，不查索引。
    assert_eq!(
        index
            .resolve_link(&journal, &link("[[#当天小结]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: journal.clone(),
            rule: ResolutionRule::ExactRelative,
        }
    );
    // 嵌套列表里的四条链接一条都不能丢。
    let document = index
        .document(&journal)
        .expect("query succeeds")
        .expect("indexed");
    assert_eq!(document.outgoing_links.len(), 4);
    assert_eq!(
        index.backlinks(&tasks).expect("backlinks succeed").len(),
        3,
        "别名、大小写与锚点三条链接都要建立反链"
    );
}

#[test]
fn text_inside_code_fences_and_inline_code_contributes_nothing_to_the_index() {
    let journal = path("项目/甲/02-日志/今天.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(
                journal.as_str(),
                "真链接 [[TASKS]]\n\n行内 `[[TASKS]]` 不算\n\n```md\n[[TASKS]]\n```\n\n转义 \\[[TASKS]] 不算",
            ),
            note("项目/甲/04-协调/TASKS.md", "# 甲任务"),
        ])
        .expect("rebuild succeeds");

    let document = index
        .document(&journal)
        .expect("query succeeds")
        .expect("indexed");
    assert_eq!(
        document
            .outgoing_links
            .iter()
            .map(|outgoing| outgoing.line)
            .collect::<Vec<_>>(),
        vec![1],
        "只有第一行那条是真链接"
    );
    assert_eq!(
        index
            .backlinks(&path("项目/甲/04-协调/TASKS.md"))
            .expect("backlinks succeed")
            .len(),
        1
    );
}

#[test]
fn resolution_results_stay_identical_as_the_vault_grows_by_two_orders_of_magnitude() {
    fn vault(total: usize) -> Vec<MarkdownDocument> {
        let mut documents: Vec<_> = (0..total)
            .map(|number| {
                note(
                    &format!("项目/{}/笔记/N{number:06}.md", number % 16),
                    &format!("# N{number:06}\n"),
                )
            })
            .collect();
        documents.push(note("项目/1/笔记/入口.md", "见 [[N000009]]"));
        documents
    }

    let from = path("项目/1/笔记/入口.md");
    let expected = LinkResolution::Resolved {
        path: path("项目/9/笔记/N000009.md"),
        rule: ResolutionRule::NearestUnique { ancestor_depth: 2 },
    };

    for total in [200_usize, 2000, 20_000] {
        let documents = vault(total);
        let mut index = InMemoryVaultIndex::new();
        let generation = index.rebuild(&documents).expect("rebuild succeeds");

        assert_eq!(generation.documents, total + 1);
        assert_eq!(generation.links, 1);
        assert_eq!(
            index
                .resolve_link(&from, &link("[[N000009]]"))
                .expect("resolution succeeds"),
            expected,
            "{total} 篇库上的解析结论变了"
        );
        assert_eq!(
            index
                .backlinks(&path("项目/9/笔记/N000009.md"))
                .expect("backlinks succeed")
                .len(),
            1
        );
    }
}

#[test]
fn link_text_must_be_spelled_in_nfc_because_a_vault_path_can_never_be_anything_else() {
    // 路径侧天然是 NFC：`studiolo_core` 直接拒收分解拼写，所以同一个文件不会有两套逻辑身份，
    // 索引的路径键与名称键也就不可能因为归一化差异而分裂。
    assert!(
        VaultPath::parse("项目/甲/cafe\u{301}.md").is_err(),
        "core 必须继续拒绝非 NFC 路径，否则索引键的前提就没了"
    );

    let journal = path("项目/甲/今天.md");
    let cafe = path("项目/甲/caf\u{e9}.md");
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            note(
                journal.as_str(),
                "组合写法 [[caf\u{e9}]]；分解写法 [[cafe\u{301}]]",
            ),
            note(cafe.as_str(), "# 咖啡"),
        ])
        .expect("rebuild succeeds");

    assert_eq!(
        index
            .resolve_link(&journal, &link("[[caf\u{e9}]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: cafe.clone(),
            rule: ResolutionRule::ExactRelative,
        },
        "NFC 拼写的链接文本必须照常命中"
    );

    // 2026-09-17 T-054 登记的缺口已由 root 接线修复：`link_index::comparison_key` 先归一到
    // NFC 再折叠大小写，正文里手写的分解拼写因此与 NFC 路径键落在同一个键上。
    assert_eq!(
        index
            .resolve_link(&journal, &link("[[cafe\u{301}]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: cafe,
            rule: ResolutionRule::ExactRelative,
        },
        "分解拼写的链接文本必须经 NFC 归一后命中同一个文件"
    );
}
