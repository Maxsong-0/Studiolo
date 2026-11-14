//! 原来源与生效策略引用只验证形状，不产生备份或外发批准。
use studiolo_core::{SourcePolicyError, SourcePolicyRef};
#[test]
fn source_policy_round_trip_retains_exact_logical_source_and_effective_digest() {
    let source = SourcePolicyRef::new("warehouse.read-v2", "ab".repeat(32)).unwrap();
    let bytes = serde_json::to_vec(&source).unwrap();
    let reopened: SourcePolicyRef = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(source, reopened);
    assert_eq!(source.source_id(), "warehouse.read-v2");
    assert_eq!(source.effective_policy_sha256(), "ab".repeat(32));
    assert!(SourcePolicyRef::new("a".repeat(128), "0".repeat(64)).is_ok());
}
#[test]
fn source_policy_rejects_paths_ambiguous_names_unknown_fields_and_non_digest_versions() {
    for id in [
        "",
        ".",
        "..",
        "/source",
        "a/b",
        "a\\b",
        "a..b",
        "a b",
        "a\0b",
        "https:source",
        "来源",
    ] {
        assert_eq!(
            SourcePolicyRef::new(id, "ab".repeat(32)),
            Err(SourcePolicyError::InvalidSource)
        );
    }
    assert!(SourcePolicyRef::new("a".repeat(129), "ab".repeat(32)).is_err());
    for digest in [
        "latest".into(),
        "7".into(),
        "AB".repeat(32),
        "ab".repeat(31),
        "ab".repeat(33),
    ] {
        assert_eq!(
            SourcePolicyRef::new("source", digest),
            Err(SourcePolicyError::InvalidPolicyDigest)
        );
    }
    for document in [
        serde_json::json!({"source_id":"source"}),
        serde_json::json!({"source_id":"a/b","effective_policy_sha256":"ab".repeat(32)}),
        serde_json::json!({"source_id":"source","effective_policy_sha256":"ab".repeat(32),"approved":true}),
    ] {
        assert!(serde_json::from_value::<SourcePolicyRef>(document).is_err());
    }
}
