//! 只限制持久编码大小，不把缓存预算变成普通索引输入限制。

use super::*;
use crate::{NormalizedFrontmatter, VaultIndex};

#[test]
fn encoder_stops_at_budget_without_affecting_a_published_index() {
    let input = MarkdownDocument::new(VaultPath::parse("页.md").unwrap(), "# 有效模型");
    let mut index = InMemoryVaultIndex::new();
    let (_, mut cache, _) = index
        .rebuild_cached(std::slice::from_ref(&input), None)
        .unwrap();
    let before = index.document(input.path()).unwrap();
    // 构造达到持久字节预算的派生数据；编码失败不能改变索引代次。
    cache.data.entries[0].parsed.frontmatter = NormalizedFrontmatter {
        raw: "x".repeat(MAX_CACHE_BYTES),
        ..NormalizedFrontmatter::default()
    };
    assert_eq!(cache.encode(), Err(CacheError::TooLarge));
    assert_eq!(index.document(input.path()).unwrap(), before);
    assert_eq!(index.generation().documents, 1);
}

#[test]
fn more_cache_rows_than_budget_do_not_reject_the_ordinary_model() {
    let documents = (0..=MAX_DOCUMENTS)
        .map(|number| {
            MarkdownDocument::new(VaultPath::parse(format!("{number:05}.md")).unwrap(), "")
        })
        .collect::<Vec<_>>();
    let mut index = InMemoryVaultIndex::new();
    let (_, cache, stats) = index.rebuild_cached(&documents, None).unwrap();
    assert_eq!(stats.parsed, MAX_DOCUMENTS + 1);
    assert_eq!(cache.encode(), Err(CacheError::TooLarge));
    assert_eq!(index.generation().documents, MAX_DOCUMENTS + 1);
}
