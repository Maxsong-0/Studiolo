//! wikilink 解析索引的公开契约之一：索引是可丢弃的派生物。
//!
//! 逐代增量维护出来的索引，必须和只拿最终文件从零重建出来的索引给出逐项相同的查询结果；
//! 退场的候选（删除、改名、别名变更、空代次）必须真正离开索引。

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

fn notes(entries: &[(&str, &str)]) -> Vec<MarkdownDocument> {
    entries
        .iter()
        .map(|(path_text, contents)| note(path_text, contents))
        .collect()
}

fn link(raw: &str) -> WikilinkTarget {
    WikilinkTarget::parse(raw).expect("valid wikilink")
}

/// 逐项比对两个索引的全部公开查询结果；generation 序号本身允许不同。
fn assert_same_views(
    incremental: &InMemoryVaultIndex,
    rebuilt: &InMemoryVaultIndex,
    paths: &[VaultPath],
) {
    assert_eq!(
        incremental.generation().documents,
        rebuilt.generation().documents
    );
    assert_eq!(incremental.generation().links, rebuilt.generation().links);
    assert_eq!(incremental.link_health(), rebuilt.link_health());
    assert_eq!(incremental.diagnostics(), rebuilt.diagnostics());
    for path in paths {
        assert_eq!(
            incremental.document(path),
            rebuilt.document(path),
            "{path} 的解析结果在两条路径上不一致"
        );
        assert_eq!(
            incremental.backlinks(path),
            rebuilt.backlinks(path),
            "{path} 的反链在两条路径上不一致"
        );
        assert_eq!(incremental.heading_tree(path), rebuilt.heading_tree(path));
        assert_eq!(
            incremental.source_profile(path),
            rebuilt.source_profile(path)
        );
    }
    // 未索引路径的失败契约也必须一致。
    let absent = path("从来没有这一篇.md");
    assert_eq!(incremental.document(&absent), rebuilt.document(&absent));
    assert_eq!(
        incremental.heading_tree(&absent).is_err(),
        rebuilt.heading_tree(&absent).is_err()
    );
}

#[test]
fn an_index_grown_generation_by_generation_matches_one_rebuilt_from_the_final_files_alone() {
    // 三条链接分别压住「就近唯一」「同名更近的一篇消失后必须改判」「精确路径消失后必须悬空」；
    // 只要有一个退场的候选留在索引里，下面的断言就会变色。
    let journal = (
        "项目/甲/02-日志/今天.md",
        "见 [[归档页]]、[[TASKS]]、[[项目/甲/04-协调/旧板]] 与 [[协调中心]]",
    );
    let first = notes(&[
        journal,
        ("项目/甲/02-日志/归档页.md", "# 归档页"),
        ("项目/甲/02-日志/TASKS.md", "# 日志里的同名页"),
        (
            "项目/甲/04-协调/TASKS.md",
            "---\naliases: [协调中心]\n---\n# 甲任务\n## 本周",
        ),
        ("项目/甲/04-协调/旧板.md", "# 旧板"),
    ]);
    // 第二代：只改别名，文档集合不变——压住「键变了就要重索引，旧键必须同时退场」。
    let second = notes(&[
        journal,
        ("项目/甲/02-日志/归档页.md", "# 归档页"),
        ("项目/甲/02-日志/TASKS.md", "# 日志里的同名页"),
        (
            "项目/甲/04-协调/TASKS.md",
            "---\naliases: [新中心]\n---\n# 甲任务\n## 本周",
        ),
        ("项目/甲/04-协调/旧板.md", "# 旧板"),
    ]);
    // 第三代：别名改回去，同时删掉三篇——每一篇的消失都会改变一条链接的结论。
    let third = notes(&[
        journal,
        (
            "项目/甲/04-协调/TASKS.md",
            "---\naliases: [协调中心]\n---\n# 甲任务\n## 本周",
        ),
    ]);

    let mut incremental = InMemoryVaultIndex::new();
    for generation in [&first, &second, &third] {
        incremental.rebuild(generation).expect("每一代都应构建成功");
    }
    let mut rebuilt = InMemoryVaultIndex::new();
    rebuilt.rebuild(&third).expect("从零重建成功");

    let mut paths: Vec<_> = third.iter().map(|input| input.path().clone()).collect();
    // 被删的三篇也要一起比：两条路径都必须认为它们已经不在索引里。
    paths.extend(
        ["项目/甲/02-日志/归档页.md", "项目/甲/04-协调/旧板.md"]
            .into_iter()
            .map(path),
    );
    assert_same_views(&incremental, &rebuilt, &paths[..third.len()]);
    for deleted in &paths[third.len()..] {
        assert_eq!(incremental.document(deleted), Ok(None));
        assert_eq!(rebuilt.document(deleted), Ok(None));
    }

    let today = path("项目/甲/02-日志/今天.md");
    // 唯一的候选被删 → 必须悬空，不能继续指向一篇不在的笔记。
    assert_eq!(
        incremental
            .resolve_link(&today, &link("[[归档页]]"))
            .expect("resolution succeeds"),
        LinkResolution::Dangling,
        "退场的候选必须真正离开索引"
    );
    // 更近的同名页被删 → 必须改判到上一层那篇，而不是停在旧结论上。
    assert_eq!(
        incremental
            .resolve_link(&today, &link("[[TASKS]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: path("项目/甲/04-协调/TASKS.md"),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        },
        "就近候选消失后必须重新按剩下的候选判定"
    );
    // 精确路径候选被删 → 必须悬空。
    assert_eq!(
        incremental
            .resolve_link(&today, &link("[[项目/甲/04-协调/旧板]]"))
            .expect("resolution succeeds"),
        LinkResolution::Dangling
    );
    // 别名绕了一圈改回来，仍然必须命中且只命中一次。
    assert_eq!(
        incremental
            .resolve_link(&today, &link("[[协调中心]]"))
            .expect("resolution succeeds"),
        LinkResolution::Resolved {
            path: path("项目/甲/04-协调/TASKS.md"),
            rule: ResolutionRule::NearestUnique { ancestor_depth: 1 },
        }
    );
    assert_eq!(
        incremental
            .resolve_link(&today, &link("[[新中心]]"))
            .expect("resolution succeeds"),
        LinkResolution::Dangling,
        "第二代用过的旧别名必须随重索引一并撤下"
    );

    let health = incremental.link_health().expect("health succeeds");
    assert_eq!(health.dangling.len(), 2, "两条链接应当悬空");
    assert!(health.ambiguous.is_empty());
}

#[test]
fn an_emptied_generation_clears_every_candidate_and_the_next_generation_rebuilds_from_files() {
    let journal = ("项目/甲/02-日志/今天.md", "见 [[TASKS]]");
    let populated = notes(&[("项目/甲/04-协调/TASKS.md", "# 甲任务"), journal]);
    // 第三代只剩日志这一篇：目标笔记在空的一代里消失后，不许被索引悄悄记住。
    let survivor = notes(&[journal]);

    let mut index = InMemoryVaultIndex::new();
    index.rebuild(&populated).expect("第一代构建成功");
    assert_eq!(index.generation().documents, 2);

    index.rebuild(&[]).expect("空的一代也必须成功");
    assert_eq!(index.generation().documents, 0);
    assert!(
        index
            .link_health()
            .expect("health succeeds")
            .dangling
            .is_empty()
    );

    index.rebuild(&survivor).expect("第三代构建成功");
    let mut rebuilt = InMemoryVaultIndex::new();
    rebuilt.rebuild(&survivor).expect("从零重建成功");

    let paths: Vec<_> = survivor.iter().map(|input| input.path().clone()).collect();
    assert_same_views(&index, &rebuilt, &paths);
    assert_eq!(
        index
            .resolve_link(&path(journal.0), &link("[[TASKS]]"))
            .expect("resolution succeeds"),
        LinkResolution::Dangling,
        "空的一代必须清空候选，之后只能由本代文件重建"
    );
}
