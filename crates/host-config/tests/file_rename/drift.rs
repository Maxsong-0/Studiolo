use super::fixture::*;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn every_pair_retains_actual_parent_config_bytes_hash_and_epoch_after_policy_drop() {
    let f = Fixture::new();
    let parent = f.policy();
    let pair = parent
        .rename_policy()
        .unwrap()
        .approve_pair(f.vault, &path("notes/a.md"), &path("other/b.md"))
        .unwrap();
    assert_eq!(pair.scope_hash(), parent.scope_hash());
    assert_eq!(pair.scope_epoch(), 7);
    drop(parent);
    pair.revalidate().unwrap();
    let mut value = f.value.clone();
    value["seed_ref"] = json!("changed-reference");
    f.write(&value);
    assert!(pair.revalidate().is_err());
    let replacement = f.policy();
    assert_ne!(pair.scope_hash(), replacement.scope_hash());
    // 内容无关字段也属于原批准；不能重载另一份配置续用旧配对。
    assert_eq!(replacement.scope_epoch(), pair.scope_epoch());
}

#[test]
fn same_bytes_replacement_links_and_permissions_invalidate_policy_and_sealed_pair() {
    for change in [
        "replace",
        "symlink",
        "hardlink",
        "file_mode",
        "directory_mode",
    ] {
        let f = Fixture::new();
        let parent = f.policy();
        let rename = parent.rename_policy().unwrap();
        let pair = rename
            .approve_pair(f.vault, &path("notes/a.md"), &path("other/b.md"))
            .unwrap();
        let bytes = fs::read(&f.path).unwrap();
        match change {
            "replace" => {
                fs::rename(&f.path, f.root.join("config/old.json")).unwrap();
                f.bytes(&bytes);
            }
            "symlink" => {
                let old = f.root.join("config/old.json");
                fs::rename(&f.path, &old).unwrap();
                std::os::unix::fs::symlink(old, &f.path).unwrap();
            }
            "hardlink" => fs::hard_link(&f.path, f.root.join("config/linked.json")).unwrap(),
            "file_mode" => fs::set_permissions(&f.path, fs::Permissions::from_mode(0o640)).unwrap(),
            _ => fs::set_permissions(f.root.join("config"), fs::Permissions::from_mode(0o750))
                .unwrap(),
        }
        assert!(parent.revalidate().is_err(), "{change}");
        assert!(rename.revalidate().is_err(), "{change}");
        assert!(pair.revalidate().is_err(), "{change}");
        assert!(
            rename
                .approve_pair(f.vault, &path("notes/a.md"), &path("other/b.md"))
                .is_err(),
            "{change}"
        );
    }
}
