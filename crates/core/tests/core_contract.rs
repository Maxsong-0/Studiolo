//! `Studiolo` 共享值类型的契约测试。

use std::str::FromStr;

use studiolo_core::{
    AttemptId, BackupScope, Classification, DataLabels, DesktopSync, Fence, ModelEgress,
    PermissionSet, Revision, TaskId, ToolId, VaultPath, VaultUuid,
};

#[test]
fn ids_round_trip_through_their_public_text_form() {
    let task = TaskId::new();
    let attempt = AttemptId::new();

    assert_eq!(TaskId::from_str(&task.to_string()).unwrap(), task);
    assert_eq!(AttemptId::from_str(&attempt.to_string()).unwrap(), attempt);
    assert!(attempt.to_string().starts_with("A-"));
}

#[test]
fn vault_paths_reject_escape_and_encoded_separators() {
    for raw in [
        "../secret.md",
        "/absolute.md",
        "notes\\secret.md",
        "notes/%2fsecret.md",
        "notes/%5Csecret.md",
        "notes/./secret.md",
        "notes//secret.md",
        "notes/secret.md/",
        "C:/secret.md",
        "notes/e\u{301}.md",
        // 分解形式的日文浊音符与 Hangul jamo 不在组合记号黑名单内，只能靠 NFC 判定拒绝。
        "notes/か\u{3099}.md",
        "docs/\u{1100}\u{1161}.md",
        "",
    ] {
        assert!(
            VaultPath::parse(raw).is_err(),
            "accepted unsafe path {raw:?}"
        );
    }

    assert_eq!(
        VaultPath::parse("项目/研究 笔记.md").unwrap().as_str(),
        "项目/研究 笔记.md"
    );
    // 组合（NFC）拼写照常接受。
    for raw in ["notes/が.md", "docs/가.md", "notes/é.md"] {
        assert!(VaultPath::parse(raw).is_ok(), "rejected NFC path {raw:?}");
    }
}

#[test]
fn protected_paths_are_classified_centrally() {
    assert!(VaultPath::parse(".git/config").unwrap().is_protected());
    assert!(
        VaultPath::parse(".workbench/vault.json")
            .unwrap()
            .is_protected()
    );
    assert!(VaultPath::parse(".stignore").unwrap().is_protected());
    assert!(!VaultPath::parse("notes/a.md").unwrap().is_protected());
}

#[test]
fn revisions_are_bound_to_vault_path_and_content() {
    let vault = VaultUuid::new();
    let path = VaultPath::parse("notes/a.md").unwrap();

    let first = Revision::compute(vault, &path, b"hello");
    assert_eq!(first, Revision::compute(vault, &path, b"hello"));
    assert_ne!(first, Revision::compute(vault, &path, b"changed"));
    assert_ne!(
        first,
        Revision::compute(vault, &VaultPath::parse("notes/b.md").unwrap(), b"hello")
    );
    assert_ne!(first, Revision::compute(VaultUuid::new(), &path, b"hello"));
}

#[test]
fn permission_sets_can_only_be_narrowed_by_intersection() {
    let broad = PermissionSet::new([
        ToolId::from_static("web.fetch"),
        ToolId::from_static("db.query"),
    ]);
    let narrow = PermissionSet::new([ToolId::from_static("web.fetch")]);

    assert_eq!(broad.intersect(&narrow), narrow);
}

#[test]
fn invalid_data_label_combinations_have_no_value() {
    let providers = ["openai".to_owned()].into_iter().collect();

    assert!(
        DataLabels::new(
            Classification::Restricted,
            BackupScope::None,
            DesktopSync::SummaryOnly,
            ModelEgress::AllowProviders(providers),
        )
        .is_err()
    );
    assert!(
        DataLabels::new(
            Classification::Public,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .is_ok()
    );
}

#[test]
fn fence_is_non_zero_and_monotonic_by_value() {
    assert!(Fence::new(0).is_none());
    assert!(Fence::new(2).unwrap() > Fence::new(1).unwrap());
}

#[test]
fn deserialization_cannot_bypass_validated_value_invariants() {
    for raw in ["../escape", "/tmp/escape", "notes/%2fescape", ""] {
        let encoded = serde_json::to_string(raw).unwrap();
        assert!(serde_json::from_str::<VaultPath>(&encoded).is_err());
    }
    let bad_labels = serde_json::json!({
        "classification": "restricted", "backup_scope": "none",
        "desktop_sync": "summary_only", "model_egress": "deny"
    });
    assert!(serde_json::from_value::<DataLabels>(bad_labels).is_err());
    assert!(serde_json::from_str::<ToolId>("\"\"").is_err());
    let path = VaultPath::parse("项目/输入.md").unwrap();
    assert_eq!(
        serde_json::from_value::<VaultPath>(serde_json::to_value(&path).unwrap()).unwrap(),
        path
    );
}

#[test]
fn attempt_json_preserves_the_public_prefixed_identity() {
    let attempt = AttemptId::new();
    assert_eq!(
        serde_json::to_value(attempt).unwrap(),
        serde_json::json!(attempt.to_string())
    );
    assert_eq!(
        serde_json::from_value::<AttemptId>(serde_json::json!(attempt.to_string())).unwrap(),
        attempt
    );
    assert!(
        serde_json::from_value::<AttemptId>(serde_json::json!(
            attempt.to_string().trim_start_matches("A-")
        ))
        .is_err()
    );
}

#[test]
fn protected_paths_reject_case_variant_spellings() {
    for raw in [
        ".GIT/config",
        ".WorkBench/vault.json",
        ".OBSIDIAN/a",
        ".STIGNORE",
    ] {
        assert!(VaultPath::parse(raw).unwrap().is_protected());
    }
}

#[test]
fn revision_bytes_roundtrip_and_match_hex() {
    let vault = VaultUuid::new();
    let path = VaultPath::parse("notes/a.md").unwrap();
    let revision = Revision::compute(vault, &path, b"hello");

    let bytes = revision.to_bytes();
    assert_eq!(Revision::from_bytes(bytes), revision);
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, revision.to_hex());
    assert_ne!(Revision::from_bytes([0; 32]), revision);
}
