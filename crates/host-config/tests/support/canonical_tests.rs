//! 新写者根批准只做只读检查；全为自有临时目录。

use super::*;
use std::os::unix::fs::{MetadataExt, symlink};

fn roots(f: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    let paths = (
        f.root.join("vault"),
        f.root.join("assets"),
        f.root.join("data"),
    );
    for path in [&paths.0, &paths.1, &paths.2] {
        private_dir(path);
    }
    paths
}

fn identity(path: &Path) -> (u64, u64) {
    let value = fs::symlink_metadata(path).unwrap();
    (value.dev(), value.ino())
}

#[test]
fn canonical_roots_keep_exact_paths_and_identities_without_initialization() {
    let f = Fixture::new();
    let (vault, assets, data) = roots(&f);
    let approved = Config::load(&f.path)
        .unwrap()
        .canonical_serving_roots(&vault, &assets, &data)
        .unwrap();
    assert_eq!(approved.vault_directory(), vault);
    assert_eq!(approved.writer_data_directory(), data);
    assert_eq!(approved.vault_identity(), identity(&vault));
    assert_eq!(approved.writer_data_identity(), identity(&data));
    assert_eq!(approved.assets_identity(), identity(&assets));
    approved.revalidate().unwrap();
    for directory in [&vault, &assets, &data] {
        assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
    }
}

#[test]
fn writer_data_cannot_overlap_any_approved_directory_in_either_direction() {
    let f = Fixture::new();
    let (vault, assets, _) = roots(&f);
    let config = Config::load(&f.path).unwrap();
    for root in [&vault, &assets, &f.root.join("auth"), &f.root.join("audit")] {
        assert!(
            config
                .canonical_serving_roots(&vault, &assets, root)
                .is_err()
        );
        let child = root.join("nested-data");
        private_dir(&child);
        assert!(
            config
                .canonical_serving_roots(&vault, &assets, &child)
                .is_err()
        );
    }
    assert!(
        config
            .canonical_serving_roots(&vault, &assets, &f.root)
            .is_err()
    );
}

#[test]
fn configuration_must_be_outside_data_but_may_share_its_parent() {
    let f = Fixture::new();
    let (vault, assets, _) = roots(&f);
    let config = Config::load(&f.path).unwrap();
    assert!(
        config
            .canonical_serving_roots(&vault, &assets, f.path.parent().unwrap())
            .is_err()
    );
    let sibling = f.path.parent().unwrap().join("broker-data");
    private_dir(&sibling);
    config
        .canonical_serving_roots(&vault, &assets, &sibling)
        .unwrap()
        .revalidate()
        .unwrap();
}

#[test]
fn data_must_already_exist_as_a_normal_private_directory() {
    let f = Fixture::new();
    let (vault, assets, data) = roots(&f);
    let config = Config::load(&f.path).unwrap();
    let alias = f.root.join("data-alias");
    symlink(&data, &alias).unwrap();
    for bad in [
        PathBuf::from("relative"),
        f.root.join("missing"),
        f.path.clone(),
        alias,
        PathBuf::from(format!("{}/./data", f.root.display())),
    ] {
        assert!(
            config
                .canonical_serving_roots(&vault, &assets, &bad)
                .is_err()
        );
    }
    assert!(!f.root.join("missing").exists());
    fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        config
            .canonical_serving_roots(&vault, &assets, &data)
            .is_err()
    );
    assert_eq!(fs::metadata(&data).unwrap().mode() & 0o7777, 0o755);
}

#[test]
fn replacement_permissions_and_config_changes_invalidate_the_original_approval() {
    for damage in ["root", "mode", "config"] {
        let f = Fixture::new();
        let (vault, assets, data) = roots(&f);
        let approved = Config::load(&f.path)
            .unwrap()
            .canonical_serving_roots(&vault, &assets, &data)
            .unwrap();
        fs::write(data.join("permitted-new-child"), b"metadata").unwrap();
        approved.revalidate().unwrap();
        match damage {
            "root" => {
                fs::rename(&data, f.root.join("old-data")).unwrap();
                private_dir(&data);
            }
            "mode" => fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap(),
            _ => {
                fs::rename(&f.path, f.root.join("config/old.json")).unwrap();
                f.write(&f.value);
            }
        }
        assert!(approved.revalidate().is_err(), "{damage}");
    }
}

#[test]
fn case_alias_overlap_uses_actual_filesystem_identity() {
    let f = Fixture::new();
    let (vault, assets, _) = roots(&f);
    let config = Config::load(&f.path).unwrap();
    let alias = f.root.join("VAULT");
    if alias.exists() {
        assert_eq!(identity(&alias), identity(&vault));
        assert!(
            config
                .canonical_serving_roots(&vault, &assets, &alias)
                .is_err()
        );
    } else {
        // 区分大小写的文件系统上这是真正分离的目录，不误判为别名。
        private_dir(&alias);
        config
            .canonical_serving_roots(&vault, &assets, &alias)
            .unwrap();
    }
}
