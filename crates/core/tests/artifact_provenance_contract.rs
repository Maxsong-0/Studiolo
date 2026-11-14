//! 原产物事实不能以反序列化绕过范围、期限、完整摘要或整组标签校验。
use serde_json::json;
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef,
};
fn fact() -> ArtifactProvenance {
    ArtifactProvenance::new(
        SourcePolicyRef::new("test-project", "a".repeat(64)).unwrap(),
        DataLabels::new(
            Classification::Restricted,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .unwrap(),
        10,
        "b".repeat(64),
    )
    .unwrap()
}
#[test]
fn original_provenance_round_trip_does_not_extend_retention() {
    let fact = fact();
    let copy: ArtifactProvenance =
        serde_json::from_slice(&serde_json::to_vec(&fact).unwrap()).unwrap();
    assert_eq!(fact, copy);
    assert!(copy.is_live_at(9_999));
    assert!(!copy.is_live_at(10_000));
    assert!(!copy.is_live_at(-1));
    assert_eq!(copy.retain_until_unix_seconds(), 10);
    assert_eq!(copy.approval_sha256(), "b".repeat(64));
}
#[test]
fn malformed_or_non_full_provenance_never_enters_full_artifact_pipeline() {
    for (field, value) in [
        ("retain_until_unix_seconds", json!(0)),
        ("retain_until_unix_seconds", json!(-1)),
        ("retain_until_unix_seconds", json!(i64::MAX)),
        ("approval_sha256", json!("latest")),
        ("approval_sha256", json!("B".repeat(64))),
        ("unknown", json!(true)),
    ] {
        let mut encoded = serde_json::to_value(fact()).unwrap();
        encoded[field] = value;
        assert!(serde_json::from_value::<ArtifactProvenance>(encoded).is_err());
    }
    for scope in ["none", "summary_only"] {
        let mut encoded = serde_json::to_value(fact()).unwrap();
        encoded["labels"]["backup_scope"] = json!(scope);
        assert!(serde_json::from_value::<ArtifactProvenance>(encoded).is_err());
    }
}
