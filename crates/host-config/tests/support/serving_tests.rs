//! 通过公开 ServingRoots Interface 验证根身份、分离与重验；不读取真实目录。

use super::*;
use std::os::unix::fs::{MetadataExt, symlink};

fn public_roots(fixture: &Fixture) -> (PathBuf, PathBuf) {
    let vault = fixture.root.join("vault");
    let assets = fixture.root.join("assets");
    for path in [&vault, &assets] {
        private_dir(path);
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    (vault, assets)
}

fn identity(path: &Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

#[test]
fn disjoint_public_roots_pin_the_real_identity_without_creating_databases() {
    let f = Fixture::new();
    let (vault, assets) = public_roots(&f);
    let config = Config::load(&f.path).unwrap();
    let serving = config.serving_roots(&vault, &assets).unwrap();
    assert_eq!(serving.vault_identity(), identity(&vault));
    assert_eq!(serving.assets_identity(), identity(&assets));
    serving.revalidate().unwrap();
    for name in ["auth", "audit", "vault", "assets"] {
        assert_eq!(fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn public_roots_cannot_equal_or_nest_each_other_or_either_private_root() {
    let f = Fixture::new();
    let (vault, assets) = public_roots(&f);
    let config = Config::load(&f.path).unwrap();
    for root in [config.auth_directory(), config.audit_directory()] {
        assert!(config.serving_roots(root, &assets).is_err());
        assert!(config.serving_roots(&vault, root).is_err());
        let nested = root.join("public-child");
        private_dir(&nested);
        assert!(config.serving_roots(&nested, &assets).is_err());
        assert!(config.serving_roots(&vault, &nested).is_err());
    }
    let nested = vault.join("assets-child");
    private_dir(&nested);
    for (left, right) in [
        (&vault, &vault),
        (&vault, &nested),
        (&nested, &vault),
        (&f.root, &assets),
    ] {
        assert!(config.serving_roots(left, right).is_err());
    }
}

#[test]
fn configuration_file_is_not_public_but_its_private_parent_may_contain_public_siblings() {
    let f = Fixture::new();
    let (_, assets) = public_roots(&f);
    let config = Config::load(&f.path).unwrap();
    let config_parent = f.path.parent().unwrap();
    assert!(config.serving_roots(config_parent, &assets).is_err());
    assert!(config.serving_roots(&assets, config_parent).is_err());
    let child = config_parent.join("public-sibling");
    private_dir(&child);
    let serving = config.serving_roots(&child, &assets).unwrap();
    serving.revalidate().unwrap();
}

#[test]
fn replacing_any_root_with_a_new_directory_is_rejected_even_when_modes_match() {
    for name in ["auth", "audit", "vault", "assets"] {
        let f = Fixture::new();
        let (vault, assets) = public_roots(&f);
        let serving = Config::load(&f.path)
            .unwrap()
            .serving_roots(&vault, &assets)
            .unwrap();
        let target = f.root.join(name);
        fs::rename(&target, f.root.join(format!("retired-{name}"))).unwrap();
        private_dir(&target);
        assert!(serving.revalidate().is_err(), "replacement: {name}");
    }
}

#[test]
fn configuration_edits_and_same_bytes_replacement_invalidate_the_saved_snapshot() {
    for replacement in [false, true] {
        let f = Fixture::new();
        let (vault, assets) = public_roots(&f);
        let config = Config::load(&f.path).unwrap();
        let serving = config.serving_roots(&vault, &assets).unwrap();
        if replacement {
            fs::rename(&f.path, f.root.join("config/retired.json")).unwrap();
            f.write(&f.value);
        } else {
            let mut value = f.value.clone();
            value["seed_ref"] = json!("another-valid-reference");
            f.write(&value);
        }
        assert!(serving.revalidate().is_err());
        assert!(config.serving_roots(&vault, &assets).is_err());
    }
}

#[test]
fn root_permission_or_symlink_changes_fail_but_new_database_children_do_not() {
    for name in ["auth", "audit"] {
        let f = Fixture::new();
        let (vault, assets) = public_roots(&f);
        let serving = Config::load(&f.path)
            .unwrap()
            .serving_roots(&vault, &assets)
            .unwrap();
        fs::set_permissions(f.root.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(serving.revalidate().is_err());
    }
    let f = Fixture::new();
    let (vault, assets) = public_roots(&f);
    let serving = Config::load(&f.path)
        .unwrap()
        .serving_roots(&vault, &assets)
        .unwrap();
    // 本契约只核验目录稳定性；子文件的真实权限与SQLite格式归各自存储Module。
    for name in ["auth", "audit"] {
        fs::write(f.root.join(name).join("synthetic-sidecar"), b"fixture").unwrap();
    }
    serving.revalidate().unwrap();
    fs::rename(&vault, f.root.join("original-vault")).unwrap();
    symlink(f.root.join("original-vault"), &vault).unwrap();
    assert!(serving.revalidate().is_err());
}

#[test]
fn public_paths_must_be_existing_normal_absolute_directories_without_links() {
    let f = Fixture::new();
    let (vault, assets) = public_roots(&f);
    let config = Config::load(&f.path).unwrap();
    let alias = f.root.join("vault-alias");
    symlink(&vault, &alias).unwrap();
    let parent_alias = f.root.join("root-alias");
    symlink(&f.root, &parent_alias).unwrap();
    for bad in [
        PathBuf::from("relative"),
        f.root.join("missing"),
        f.path.clone(),
        alias,
        parent_alias.join("vault"),
        PathBuf::from(format!("{}/./vault", f.root.display())),
        PathBuf::from(format!("{}//vault", f.root.display())),
        PathBuf::from(format!("{}/vault/", f.root.display())),
    ] {
        assert!(config.serving_roots(&bad, &assets).is_err());
        assert!(config.serving_roots(&vault, &bad).is_err());
    }
    assert!(!f.root.join("missing").exists());
}

#[test]
fn configuration_file_permissions_and_new_hardlinks_are_rechecked() {
    for hardlink in [false, true] {
        let f = Fixture::new();
        let (vault, assets) = public_roots(&f);
        let serving = Config::load(&f.path)
            .unwrap()
            .serving_roots(&vault, &assets)
            .unwrap();
        if hardlink {
            fs::hard_link(&f.path, f.root.join("config/second-link.json")).unwrap();
        } else {
            fs::set_permissions(&f.path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(serving.revalidate().is_err());
    }
}
