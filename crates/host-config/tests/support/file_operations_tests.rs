//! 默认关闭和整文件范围须由真实私有配置决定；不写实际Vault。
use super::*;
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_host_config::FileOperationPolicy;

fn setup() -> (Fixture, Value, VaultUuid) {
    let f = Fixture::new();
    let vault = VaultUuid::new();
    let mut value = f.value.clone();
    value["human_content"] = json!({"version":1,"vault_uuid":vault,"folders":["notes"],
        "protected":["notes/private"],"documents":{
            "notes/Generated.md":[["<!-- start -->","<!-- end -->"]],
            "project/TASKS.md":[],"plain.md":[]}});
    value["file_operations"] = json!({"version":1,"vault_uuid":vault,"scope_epoch":1,
        "effective_object_limit":268435456,"max_tombstones":64,
        "retained_bytes_limit":1073741824_u64,"minimum_free_bytes":16777216});
    f.write(&value);
    (f, value, vault)
}
fn policy(f: &Fixture) -> FileOperationPolicy {
    Config::load(&f.path)
        .unwrap()
        .file_operation_policy()
        .unwrap()
        .unwrap()
}
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[test]
fn file_operations_default_off_and_no_implicit_enable_from_editing() {
    let f = Fixture::new();
    let config = Config::load(&f.path).unwrap();
    assert!(config.file_operation_policy().unwrap().is_none());
    let (f, mut value, _) = setup();
    value.as_object_mut().unwrap().remove("file_operations");
    f.write(&value);
    assert!(
        Config::load(&f.path)
            .unwrap()
            .file_operation_policy()
            .unwrap()
            .is_none()
    );
    for name in ["auth", "audit"] {
        assert_eq!(fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn complete_policy_preserves_explicit_budgets_and_matching_vault_only() {
    let (f, _, vault) = setup();
    let policy = policy(&f);
    assert_eq!(policy.vault_uuid(), vault);
    assert_eq!(policy.scope_epoch(), 1);
    assert_eq!(policy.effective_object_limit(), 268435456);
    assert_eq!(policy.max_tombstones(), 64);
    assert_eq!(policy.retained_bytes_limit(), 1073741824);
    assert_eq!(policy.minimum_free_bytes(), 16777216);
    assert!(policy.allows(vault, &path("notes/plain.md")).unwrap());
    assert!(policy.allows(vault, &path("plain.md")).unwrap());
    assert!(
        !policy
            .allows(VaultUuid::new(), &path("notes/plain.md"))
            .unwrap()
    );
    assert!(!f.root.join("notes").exists());
}

#[test]
fn generated_reserved_and_raw_paths_never_gain_whole_file_permission() {
    let (f, _, vault) = setup();
    let policy = policy(&f);
    for name in [
        "notes/Generated.md",
        "notes/generated.md",
        "notes/Generated.md/child",
        "notes/private/a.md",
        "notes/PRIVATE/a.md",
        "notes/CLAUDE.md",
        "notes/ops/a.md",
        "notes/raw/a.md",
        "notes/.key",
        "notes/INDEX.md",
        "notes/TASKS.md",
        "project/TASKS.md",
        "notes-other/a.md",
        "elsewhere/a.md",
    ] {
        assert!(!policy.allows(vault, &path(name)).unwrap(), "{name}");
    }
    // 普通部分编辑权限本身未被新API改变。
    for name in ["vault", "assets", "data"] {
        private_dir(&f.root.join(name));
    }
    let roots = Config::load(&f.path)
        .unwrap()
        .canonical_serving_roots(
            &f.root.join("vault"),
            &f.root.join("assets"),
            &f.root.join("data"),
        )
        .unwrap();
    assert!(
        roots
            .human_content()
            .unwrap()
            .allows(vault, &path("project/TASKS.md"))
    );
    assert!(
        !roots
            .file_operation_policy()
            .unwrap()
            .allows(vault, &path("project/TASKS.md"))
            .unwrap()
    );
}

#[test]
fn unknown_missing_duplicate_and_nonobject_policy_fields_are_rejected() {
    let (f, value, _) = setup();
    for bad in [
        Value::Null,
        json!(false),
        json!([]),
        json!([1, "vault", 1, 1, 1, 1, 1]),
    ] {
        let mut candidate = value.clone();
        candidate["file_operations"] = bad;
        f.rejects(&candidate);
    }
    for key in [
        "version",
        "vault_uuid",
        "scope_epoch",
        "effective_object_limit",
        "max_tombstones",
        "retained_bytes_limit",
        "minimum_free_bytes",
    ] {
        let mut candidate = value.clone();
        candidate["file_operations"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        f.rejects(&candidate);
    }
    let mut candidate = value.clone();
    candidate["file_operations"]["recursive"] = json!(true);
    f.rejects(&candidate);
    let encoded = serde_json::to_string(&value).unwrap();
    f.bytes(
        encoded
            .replacen(
                "\"scope_epoch\":1",
                "\"scope_epoch\":1,\"scope_epoch\":1",
                1,
            )
            .as_bytes(),
    );
    assert!(Config::load(&f.path).is_err());
}

#[test]
fn policy_requires_existing_human_scope_same_vault_and_fixed_positive_limits() {
    let (f, value, _) = setup();
    let mut candidate = value.clone();
    candidate.as_object_mut().unwrap().remove("human_content");
    f.rejects(&candidate);
    let mut candidate = value.clone();
    candidate["file_operations"]["vault_uuid"] = json!(VaultUuid::new());
    f.rejects(&candidate);
    for (key, bad) in [
        ("version", json!(2)),
        ("scope_epoch", json!(0)),
        ("scope_epoch", json!(u64::MAX)),
        ("effective_object_limit", json!(0)),
        ("effective_object_limit", json!(268435457)),
        ("max_tombstones", json!(0)),
        ("max_tombstones", json!(4097)),
        ("retained_bytes_limit", json!(0)),
        ("retained_bytes_limit", json!(2199023255553_u64)),
        ("minimum_free_bytes", json!(0)),
    ] {
        let mut candidate = value.clone();
        candidate["file_operations"][key] = bad;
        f.rejects(&candidate);
    }
}

#[test]
fn hash_binds_domain_and_actual_complete_configuration_including_unrelated_fields() {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let (f, mut value, vault) = setup();
    let original = policy(&f);
    let bytes = fs::read(&f.path).unwrap();
    let mut expected = Sha256::new();
    expected.update(b"studiolo.human-file-operations.private-config.v1\0");
    expected.update((bytes.len() as u64).to_be_bytes());
    expected.update(&bytes);
    let mut expected_hex = String::new();
    for byte in expected.finalize() {
        write!(&mut expected_hex, "{byte:02x}").unwrap();
    }
    assert_eq!(original.scope_hash(), expected_hex);
    value["seed_ref"] = json!("other-seed-reference");
    f.write(&value);
    assert!(original.revalidate().is_err());
    assert!(original.allows(vault, &path("notes/a.md")).is_err());
    assert_ne!(original.scope_hash(), policy(&f).scope_hash());
}

#[test]
fn same_bytes_file_replacement_and_public_roots_keep_original_policy_invalidated() {
    let (f, _, vault) = setup();
    for name in ["vault", "assets", "data"] {
        private_dir(&f.root.join(name));
    }
    let roots = Config::load(&f.path)
        .unwrap()
        .canonical_serving_roots(
            &f.root.join("vault"),
            &f.root.join("assets"),
            &f.root.join("data"),
        )
        .unwrap();
    let bytes = fs::read(&f.path).unwrap();
    fs::rename(&f.path, f.root.join("config/old.json")).unwrap();
    f.bytes(&bytes);
    assert!(roots.revalidate().is_err());
    assert!(
        roots
            .file_operation_policy()
            .unwrap()
            .allows(vault, &path("notes/a.md"))
            .is_err()
    );
    assert!(policy(&f).allows(vault, &path("notes/a.md")).unwrap());
}
