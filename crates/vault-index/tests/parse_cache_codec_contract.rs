//! 坏缓存仅是 miss；不扩大普通输入限制，不容忍不认识的持久结构。

use studiolo_core::VaultPath;
use studiolo_vault_index::{
    CacheError, InMemoryVaultIndex, MAX_CACHE_BYTES, MarkdownDocument, ParseCache,
};

fn cache_json() -> serde_json::Value {
    let mut index = InMemoryVaultIndex::new();
    let documents = [MarkdownDocument::new(
        VaultPath::parse("页.md").unwrap(),
        "---\ntitle: 页\n---\n# 页\n正文 [[目标]]",
    )];
    let (_, cache, _) = index.rebuild_cached(&documents, None).unwrap();
    serde_json::from_slice(&cache.encode().unwrap()).unwrap()
}

#[test]
fn unknown_schema_or_parser_versions_are_discardable() {
    for field in ["schema_version", "parser_version"] {
        let mut value = cache_json();
        value[field] = 999.into();
        assert!(matches!(
            ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
            Err(CacheError::UnsupportedVersion)
        ));
    }
}

#[test]
fn truncated_unknown_and_duplicate_envelope_fields_are_rejected() {
    let mut value = cache_json();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        ParseCache::decode(&bytes[..bytes.len() - 1]),
        Err(CacheError::InvalidEncoding)
    ));
    value["body"] = "不能恢复正文".into();
    assert!(matches!(
        ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
        Err(CacheError::InvalidEncoding)
    ));
    let duplicate =
        b"{\"schema_version\":1,\"schema_version\":1,\"parser_version\":1,\"entries\":[]}";
    assert!(matches!(
        ParseCache::decode(duplicate),
        Err(CacheError::InvalidEncoding)
    ));
}

#[test]
fn invalid_paths_hashes_locations_and_duplicate_rows_are_rejected() {
    for (field, replacement) in [("path", "../秘密.md"), ("content_sha256", "garbled")] {
        let mut value = cache_json();
        value["entries"][0][field] = replacement.into();
        assert!(ParseCache::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut value = cache_json();
    value["entries"][0]["parsed"]["headings"][0]["level"] = 255.into();
    assert!(matches!(
        ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
        Err(CacheError::InvalidRecord)
    ));
    let mut value = cache_json();
    let duplicate = value["entries"][0].clone();
    value["entries"].as_array_mut().unwrap().push(duplicate);
    assert!(matches!(
        ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
        Err(CacheError::InvalidRecord)
    ));
}

#[test]
fn cached_cross_document_resolution_field_is_rejected() {
    let mut value = cache_json();
    value["entries"][0]["parsed"]["resolution"] = "不接受旧代次链接结论".into();
    assert!(matches!(
        ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
        Err(CacheError::InvalidEncoding)
    ));
}

#[test]
fn nested_derived_structures_reject_unknown_fields() {
    for pointer in [
        "/entries/0",
        "/entries/0/parsed/frontmatter",
        "/entries/0/parsed/frontmatter/entries/0",
        "/entries/0/parsed/headings/0",
        "/entries/0/parsed/links/0",
        "/entries/0/parsed/links/0/target",
    ] {
        let mut value = cache_json();
        value.pointer_mut(pointer).unwrap()["unknown"] = true.into();
        assert!(matches!(
            ParseCache::decode(&serde_json::to_vec(&value).unwrap()),
            Err(CacheError::InvalidEncoding)
        ));
    }
}

#[test]
fn decode_budget_is_checked_before_json_parsing() {
    assert!(matches!(
        ParseCache::decode(&vec![b' '; MAX_CACHE_BYTES + 1]),
        Err(CacheError::TooLarge)
    ));
}
