//! 公开索引契约：有界 Unicode 上下文、准确列号、唯一标题锚点与解析缓存失效。

use std::collections::BTreeSet;

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    CacheError, InMemoryVaultIndex, IndexedDocument, MarkdownDocument, PARSE_CACHE_VERSION,
    ParseCache, VaultIndex,
};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn indexed(body: &str) -> (InMemoryVaultIndex, IndexedDocument) {
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            MarkdownDocument::new(path("源.md"), body),
            MarkdownDocument::new(path("目标.md"), "# 目标"),
        ])
        .unwrap();
    let document = index.document(&path("源.md")).unwrap().unwrap();
    (index, document)
}

#[test]
fn short_lines_including_the_exact_unicode_limit_are_unchanged() {
    let short = "中😀 [[目标]] 和 ![[目标|显示]]";
    let boundary = format!("中 [[目标]]{}", "😀".repeat(312));
    assert_eq!(boundary.chars().count(), 320);
    let (_, document) = indexed(&format!("{short}\n{boundary}"));
    assert_eq!(document.outgoing_links.len(), 3);
    assert_eq!(document.outgoing_links[0].context, short);
    assert_eq!(document.outgoing_links[1].context, short);
    assert_eq!(document.outgoing_links[2].context, boundary);
    assert_eq!(document.outgoing_links[0].column, 4);
    assert_eq!(document.outgoing_links[2].column, 3);

    let above_limit = format!("{boundary}尾");
    let (_, document) = indexed(&above_limit);
    let context = &document.outgoing_links[0].context;
    assert_eq!(context.chars().count(), 320);
    assert!(context.ends_with('…'));
    assert_eq!(
        context,
        &format!("{}…", boundary.chars().take(319).collect::<String>())
    );
}

#[test]
fn long_unicode_lines_keep_exact_positions_and_bounded_link_windows() {
    let tokens = ["[[目标|开头]]", "[[目标]]", "![[目标]]", "[[目标|结尾]]"];
    let line = format!(
        "{}{} `[[隐藏]]` \\[[转义]] [[ ]] {}{}{}{}{}",
        tokens[0],
        "中😀".repeat(5000),
        tokens[1],
        "文🌱".repeat(300),
        tokens[2],
        "语🦀".repeat(300),
        tokens[3],
    );
    let (index, document) = indexed(&format!("---\ntitle: 来源\n---\n{line}"));
    assert_eq!(document.outgoing_links.len(), tokens.len());
    for (link, token) in document.outgoing_links.iter().zip(tokens) {
        let start = line.find(token).unwrap();
        assert_eq!(link.line, 4);
        assert_eq!(link.column as usize, line[..start].chars().count() + 1);
        assert!(link.context.chars().count() <= 320);
        assert!(link.context.contains(token));
        if start > 0 {
            let prefix = line[..start].chars().rev().take(32).collect::<Vec<_>>();
            let prefix = prefix.into_iter().rev().collect::<String>();
            assert!(link.context.starts_with(&format!("…{prefix}{token}")));
        }
    }
    assert!(!document.outgoing_links[0].context.starts_with('…'));
    assert!(document.outgoing_links[0].context.ends_with('…'));
    assert!(document.outgoing_links[2].target.embed);
    assert!(document.outgoing_links[3].context.ends_with(tokens[3]));
    let backlinks = index.backlinks(&path("目标.md")).unwrap();
    assert_eq!(backlinks.len(), tokens.len());
    for (backlink, link) in backlinks.iter().zip(&document.outgoing_links) {
        assert_eq!(backlink.context, link.context);
        assert_eq!((backlink.line, backlink.column), (link.line, link.column));
    }
}

#[test]
fn dense_unicode_links_do_not_duplicate_the_entire_source_line() {
    let unit = "汉😀[[目标]] ";
    let (_, document) = indexed(&unit.repeat(1024));
    assert_eq!(document.outgoing_links.len(), 1024);
    for (offset, link) in document.outgoing_links.iter().enumerate() {
        assert_eq!(link.column as usize, offset * unit.chars().count() + 3);
        assert!(link.context.chars().count() <= 320);
    }
    assert!(
        document
            .outgoing_links
            .iter()
            .map(|link| link.context.chars().count())
            .sum::<usize>()
            <= 1024 * 320
    );
}

#[test]
fn heading_anchors_are_unique_with_explicit_suffixes_and_empty_slugs() {
    let cases = [
        ("# A\n# A\n# A-1", vec!["a", "a-1", "a-1-1"]),
        ("# A-1\n# A\n# A", vec!["a-1", "a", "a-2"]),
        (
            "# A\n# A-1\n# A\n# A-2\n# A",
            vec!["a", "a-1", "a-2", "a-2-1", "a-3"],
        ),
        (
            "# !!!\n# ??\n# section\n# section-1\n# !",
            vec![
                "section",
                "section-1",
                "section-2",
                "section-1-1",
                "section-3",
            ],
        ),
        (
            "# Hello World\n# 中文 标题\n# Hello World",
            vec!["hello-world", "中文-标题", "hello-world-1"],
        ),
    ];
    for (body, expected) in cases {
        let (_, document) = indexed(body);
        let anchors = document
            .headings
            .iter()
            .map(|heading| heading.anchor.as_str())
            .collect::<Vec<_>>();
        assert_eq!(anchors, expected, "{body:?}");
        assert!(anchors.iter().all(|anchor| !anchor.is_empty()));
        assert_eq!(anchors.iter().collect::<BTreeSet<_>>().len(), anchors.len());
    }
}

#[test]
fn parser_version_one_is_rejected_and_current_cache_preserves_new_semantics() {
    let body = format!("# A\n# A\n# A-1\n{}[[目标]]", "中😀".repeat(300));
    let documents = [MarkdownDocument::new(path("源.md"), &body)];
    let mut cold = InMemoryVaultIndex::new();
    let (_, cache, stats) = cold.rebuild_cached(&documents, None).unwrap();
    assert_eq!((stats.parsed, stats.reused), (1, 0));
    assert_eq!(PARSE_CACHE_VERSION, 2);
    let encoded = cache.encode().unwrap();
    let mut old: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    old["parser_version"] = 1.into();
    assert!(matches!(
        ParseCache::decode(&serde_json::to_vec(&old).unwrap()),
        Err(CacheError::UnsupportedVersion)
    ));
    let decoded = ParseCache::decode(&encoded).unwrap();
    let mut warm = InMemoryVaultIndex::new();
    let (_, _, stats) = warm.rebuild_cached(&documents, Some(&decoded)).unwrap();
    assert_eq!((stats.parsed, stats.reused), (0, 1));
    assert_eq!(
        warm.document(&path("源.md")).unwrap(),
        cold.document(&path("源.md")).unwrap()
    );
}
