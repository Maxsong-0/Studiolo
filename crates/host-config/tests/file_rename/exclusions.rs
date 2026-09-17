use super::fixture::*;
use serde_json::json;
use studiolo_core::VaultUuid;
use studiolo_host_config::Config;

#[test]
fn rename_rules_reject_overlap_aliases_and_any_protected_or_generated_region() {
    let f = Fixture::new();
    for folders in [
        json!([rule("notes", 100), rule("notes/sub", 100)]),
        json!([rule("notes", 100), rule("notes", 100)]),
        json!([rule("notes/a", 100), rule("notes/A/b", 100)]),
        json!([rule("notes/Straße/a", 100), rule("notes/STRASSE/b", 100)]),
        json!([rule("blocked", 100)]),
        json!([rule("generated/auto.md", 100)]),
        json!([rule("notes/raw", 100)]),
        json!([rule("notes/.private", 100)]),
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_folders"] = folders;
        f.rejects(&value);
    }
    for file in [
        "notes/a.md",
        "notes",
        "notes/TASKS.md",
        "generated/AUTO.md",
        "blocked/a.md",
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"]["whole_files"] = json!([rule(file, 100)]);
        f.rejects(&value);
    }
    for (field, value) in [
        ("protected", json!(["notes/private"])),
        (
            "documents",
            json!({"notes/auto.md":[["<!-- start -->","<!-- end -->"]]}),
        ),
    ] {
        let mut candidate = f.value.clone();
        candidate["human_content"][field] = value;
        f.rejects(&candidate);
    }
}

#[test]
fn sync_only_generated_registration_denies_all_whole_file_actions_without_granting_sync() {
    let f = Fixture::new();
    let mut value = f.value.clone();
    value["sync_scope"] = sync(&f, &["notes/Only-Sync.md"]);
    value["file_operations"]
        .as_object_mut()
        .unwrap()
        .remove("rename");
    f.write(&value);
    let parent = f.policy();
    for action in ["delete", "restore_tombstone"] {
        for name in [
            "notes/Only-Sync.md",
            "notes/only-sync.md",
            "notes/Only-Sync.md/child.md",
        ] {
            assert!(
                !parent.allows(f.vault, &path(name)).unwrap(),
                "{action}: {name}"
            );
        }
    }
    assert!(parent.allows(f.vault, &path("notes/manual.md")).unwrap());
    assert!(parent.rename_policy().is_none());
    // Rename 的安全精确规则可加载；被 Sync 登记的路径仍不能组成配对。
    value["file_operations"]["rename"] = json!({"version":1,"whole_folders":[],
        "whole_files":[rule("notes/a.md",100),rule("notes/b.md",100)]});
    f.write(&value);
    let parent = f.policy();
    let rename = parent.rename_policy().unwrap();
    assert!(
        rename
            .approve_pair(f.vault, &path("notes/Only-Sync.md"), &path("notes/b.md"))
            .is_err()
    );
    assert!(
        rename
            .approve_pair(f.vault, &path("notes/a.md"), &path("notes/only-sync.md"))
            .is_err()
    );
    assert!(
        rename
            .approve_pair(f.vault, &path("notes/a.md"), &path("notes/b.md"))
            .is_ok()
    );
    assert!(!parent.allows(f.vault, &path("sync/input.md")).unwrap());
    // 原目录规则覆盖了禁止区域，必须拒绝整个配置，不能静默挖洞。
    value["file_operations"]["rename"] = f.value["file_operations"]["rename"].clone();
    f.rejects(&value);
}

#[test]
fn generated_guards_ignore_content_markers_and_cannot_cross_vault_or_missing_human_scope() {
    let f = Fixture::new();
    for generated in ["notes/Auto.md", "notes/auto.md", "notes/auto.md/sub"] {
        let mut value = f.value.clone();
        value["sync_scope"] = sync(&f, &[generated]);
        f.rejects(&value);
    }
    let mut value = f.value.clone();
    value["sync_scope"] = sync(&f, &["notes/auto.md"]);
    value["file_operations"]
        .as_object_mut()
        .unwrap()
        .remove("rename");
    value["sync_scope"]["vault_uuid"] = json!(VaultUuid::new());
    f.rejects(&value);
    value = f.value.clone();
    value.as_object_mut().unwrap().remove("human_content");
    f.rejects(&value);
    value = f.value.clone();
    value["file_operations"]["vault_uuid"] = json!(VaultUuid::new());
    f.rejects(&value);
    assert!(!f.root.join("notes").exists());
    // 非 NFC 或不可见拼写在配置解码/同一索引阶段拒绝，不需要实际文件系统来“纠正”。
    for name in [
        "notes/e\u{301}.md",
        "notes/a\u{200b}.md",
        "notes/a\u{fe0f}.md",
        "notes/a:b.md",
    ] {
        let mut value = f.value.clone();
        value["file_operations"]["rename"] =
            json!({"version":1,"whole_folders":[],"whole_files":[rule(name,100)]});
        f.write(&value);
        assert!(Config::load(&f.path).is_err());
    }
}
