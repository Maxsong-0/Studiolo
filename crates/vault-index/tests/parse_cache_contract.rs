//! 通过公开接口验证缓存可丢弃、精确源绑定和跨文档重新解析。

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, MarkdownDocument, ParseCache, ParseCacheStats, VaultIndex,
};

fn note(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

fn build(
    documents: &[MarkdownDocument],
    previous: Option<&ParseCache>,
) -> (InMemoryVaultIndex, ParseCache, ParseCacheStats) {
    let mut index = InMemoryVaultIndex::new();
    let (_, cache, stats) = index.rebuild_cached(documents, previous).unwrap();
    (index, cache, stats)
}

fn assert_same(
    left: &InMemoryVaultIndex,
    right: &InMemoryVaultIndex,
    documents: &[MarkdownDocument],
) {
    assert_eq!(left.generation(), right.generation());
    assert_eq!(left.link_health(), right.link_health());
    for document in documents {
        let path = document.path();
        assert_eq!(left.document(path), right.document(path));
        assert_eq!(left.heading_tree(path), right.heading_tree(path));
        assert_eq!(left.backlinks(path), right.backlinks(path));
        assert_eq!(left.source_profile(path), right.source_profile(path));
    }
}

#[test]
fn cold_and_warm_cache_match_ordinary_build_without_reparsing() {
    let documents = vec![
        note(
            "项目/甲/入口.md",
            "---\n标题: 未映射\nstatus: done\n状态: 草稿\n---\n# 入口\n## 次级\n[[目标|别名]] [[不存在]]\n",
        ),
        note(
            "项目/甲/目标.md",
            "---\ntitle: 目标\naliases: [别称]\nsource: 外部声明\n---\n# 正文\n只存在输入中的普通正文。\n",
        ),
        note("警告.md", "---\n未闭合: true\n"),
    ];
    let mut ordinary = InMemoryVaultIndex::new();
    ordinary.rebuild(&documents).unwrap();
    let (cold, cache, stats) = build(&documents, None);
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 0,
            parsed: 3
        }
    );
    let bytes = cache.encode().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains("只存在输入中的普通正文"));
    for forbidden in [
        "resolution",
        "backlinks",
        "freshness",
        "principal",
        "source_profile",
    ] {
        assert!(
            !text.contains(forbidden),
            "缓存含不应持久化的字段 {forbidden}"
        );
    }
    let decoded = ParseCache::decode(&bytes).unwrap();
    let (warm, next, stats) = build(&documents, Some(&decoded));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 3,
            parsed: 0
        }
    );
    assert_eq!(bytes, next.encode().unwrap());
    assert_same(&ordinary, &cold, &documents);
    assert_same(&ordinary, &warm, &documents);
}

#[test]
fn exact_bytes_and_path_both_participate_in_cache_identity() {
    let (_, cache, _) = build(&[note("页.md", "# A\n")], None);
    for changed in [
        note("页.md", "# B\n"),
        note("页.md", "# A\r\n"),
        note("另页.md", "# A\n"),
    ] {
        let (_, _, stats) = build(&[changed], Some(&cache));
        assert_eq!(
            stats,
            ParseCacheStats {
                reused: 0,
                parsed: 1
            }
        );
    }
}

#[test]
fn duplicate_input_does_not_replace_a_published_generation() {
    let original = note("页.md", "# 原始");
    let (mut index, cache, _) = build(std::slice::from_ref(&original), None);
    let before = index.document(original.path()).unwrap();
    let generation = index.generation();
    assert!(
        index
            .rebuild_cached(&[original.clone(), note("页.md", "# 重复")], Some(&cache))
            .is_err()
    );
    assert_eq!(index.generation(), generation);
    assert_eq!(index.document(original.path()).unwrap(), before);
}

#[test]
fn extra_decoded_cache_path_never_enters_the_current_input_set() {
    let current = note("页.md", "# 现有");
    let (_, cache, _) = build(std::slice::from_ref(&current), None);
    let mut serialized: serde_json::Value =
        serde_json::from_slice(&cache.encode().unwrap()).unwrap();
    let mut extra = serialized["entries"][0].clone();
    extra["path"] = "另一个权限域/泄漏.md".into();
    serialized["entries"]
        .as_array_mut()
        .unwrap()
        .insert(0, extra);
    let decoded = ParseCache::decode(&serde_json::to_vec(&serialized).unwrap()).unwrap();
    let (index, cache, stats) = build(&[current], Some(&decoded));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 1,
            parsed: 0
        }
    );
    assert_eq!(index.generation().documents, 1);
    assert!(
        index
            .document(&VaultPath::parse("另一个权限域/泄漏.md").unwrap())
            .unwrap()
            .is_none()
    );
    let serialized: serde_json::Value = serde_json::from_slice(&cache.encode().unwrap()).unwrap();
    assert_eq!(serialized["entries"].as_array().unwrap().len(), 1);
}
