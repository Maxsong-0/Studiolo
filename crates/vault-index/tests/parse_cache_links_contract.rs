//! 单篇复用不得冻结跨文档链接目标。

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    InMemoryVaultIndex, LinkResolution, MarkdownDocument, ParseCache, ParseCacheStats, VaultIndex,
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

fn resolution(index: &InMemoryVaultIndex, path: &str) -> LinkResolution {
    index
        .document(&VaultPath::parse(path).unwrap())
        .unwrap()
        .unwrap()
        .outgoing_links[0]
        .resolution
        .clone()
}

#[test]
fn changed_alias_reparses_one_note_and_rebuilds_inbound_links() {
    let documents = vec![
        note("入口.md", "[[别称]]"),
        note("目标.md", "---\naliases: [别称]\n---\n# 页\n"),
    ];
    let (first, cache, _) = build(&documents, None);
    assert!(matches!(
        resolution(&first, "入口.md"),
        LinkResolution::Resolved { .. }
    ));
    let changed = vec![
        documents[0].clone(),
        note("目标.md", "---\naliases: [其他]\n---\n# 页\n"),
    ];
    let (next, _, stats) = build(&changed, Some(&cache));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 1,
            parsed: 1
        }
    );
    assert_eq!(resolution(&next, "入口.md"), LinkResolution::Dangling);
    assert!(
        next.backlinks(&VaultPath::parse("目标.md").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn new_target_can_resolve_a_cached_dangling_link_and_new_name_can_make_it_ambiguous() {
    let source = note("入口.md", "[[目标]]");
    let (first, cache, _) = build(std::slice::from_ref(&source), None);
    assert_eq!(resolution(&first, "入口.md"), LinkResolution::Dangling);
    let documents = vec![source.clone(), note("甲/目标.md", "# 甲")];
    let (next, cache, stats) = build(&documents, Some(&cache));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 1,
            parsed: 1
        }
    );
    assert!(matches!(
        resolution(&next, "入口.md"),
        LinkResolution::Resolved { .. }
    ));
    let ambiguous = vec![source, documents[1].clone(), note("乙/目标.md", "# 乙")];
    let (next, _, stats) = build(&ambiguous, Some(&cache));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 2,
            parsed: 1
        }
    );
    assert!(
        matches!(resolution(&next, "入口.md"), LinkResolution::Ambiguous { candidates } if candidates.len() == 2)
    );
}

#[test]
fn deletion_and_rename_remove_old_paths_and_rebuild_resolutions() {
    let source = note("入口.md", "[[目标]]");
    let (_, cache, _) = build(&[source.clone(), note("目标.md", "# 页")], None);
    let (deleted, next, stats) = build(std::slice::from_ref(&source), Some(&cache));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 1,
            parsed: 0
        }
    );
    assert_eq!(resolution(&deleted, "入口.md"), LinkResolution::Dangling);
    assert!(
        deleted
            .document(&VaultPath::parse("目标.md").unwrap())
            .unwrap()
            .is_none()
    );
    let serialized: serde_json::Value = serde_json::from_slice(&next.encode().unwrap()).unwrap();
    assert_eq!(serialized["entries"].as_array().unwrap().len(), 1);
    let (renamed, _, stats) = build(&[source, note("改名.md", "# 页")], Some(&cache));
    assert_eq!(
        stats,
        ParseCacheStats {
            reused: 1,
            parsed: 1
        }
    );
    assert_eq!(resolution(&renamed, "入口.md"), LinkResolution::Dangling);
}
