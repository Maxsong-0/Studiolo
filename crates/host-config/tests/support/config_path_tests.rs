//! 迁移原管理命令的真实路径契约；只验证指定配置和存储根，不声称隔离整个目录树。

use super::*;
use std::os::unix::{
    fs::{MetadataExt, symlink},
    net::UnixListener,
};

#[test]
fn private_config_requires_current_owner_single_link_and_exact_permissions() {
    let f = Fixture::new();
    assert_eq!(
        fs::metadata(&f.path).unwrap().uid(),
        rustix::process::getuid().as_raw()
    );
    for mode in [0o644, 0o400, 0o700, 0o1600] {
        fs::set_permissions(&f.path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(Config::load(&f.path).is_err());
        assert_eq!(fs::metadata(&f.path).unwrap().mode() & 0o7777, mode);
    }
    fs::set_permissions(&f.path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&f.path, f.root.join("config-alias")).unwrap();
    assert!(Config::load(&f.path).is_err());
    assert_eq!(fs::metadata(&f.path).unwrap().nlink(), 2);
}

#[test]
fn config_symlink_missing_and_special_nodes_are_not_read_or_created() {
    let f = Fixture::new();
    let alias = f.root.join("config/alias.json");
    symlink(&f.path, &alias).unwrap();
    assert!(Config::load(&alias).is_err());
    let absent = f.root.join("config/absent.json");
    assert!(Config::load(&absent).is_err());
    assert!(!absent.exists());
    let socket = f.root.join("config/socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    assert!(Config::load(&socket).is_err());
    assert!(Config::load(Path::new("relative.json")).is_err());
    assert!(Config::load(f.path.parent().unwrap()).is_err());
}

#[test]
fn every_selected_private_directory_must_already_exist_with_mode_seven_hundred() {
    for name in ["config", "auth", "audit"] {
        let f = Fixture::new();
        let target = f.root.join(name);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Config::load(&f.path).is_err());
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o755);
    }
    let f = Fixture::new();
    for key in ["auth_directory", "audit_directory"] {
        let mut config = f.value.clone();
        let missing = f.root.join("not-created");
        config[key] = json!(missing);
        f.rejects(&config);
        assert!(!missing.exists());
        config[key] = json!(f.path);
        f.rejects(&config);
    }
}

#[test]
fn overlapping_relative_control_and_parent_traversal_roots_are_rejected() {
    let f = Fixture::new();
    let auth = f.root.join("auth");
    let nested = auth.join("nested");
    private_dir(&nested);
    for (left, right) in [(&auth, &auth), (&auth, &nested), (&nested, &auth)] {
        let mut config = f.value.clone();
        config["auth_directory"] = json!(left);
        config["audit_directory"] = json!(right);
        f.rejects(&config);
    }
    for path in [
        "relative".to_owned(),
        format!("{}/auth/../audit", f.root.display()),
        format!("{}/bad\npath", f.root.display()),
    ] {
        let mut config = f.value.clone();
        config["auth_directory"] = json!(path);
        f.rejects(&config);
    }
    // 共同文本前缀不是父子关系，合法同级目录不能被误拒。
    let sibling = f.root.join("auth-sibling");
    private_dir(&sibling);
    let mut config = f.value.clone();
    config["audit_directory"] = json!(sibling);
    f.write(&config);
    assert!(Config::load(&f.path).is_ok());
}

#[test]
fn symlink_in_config_or_storage_ancestors_is_rejected() {
    let f = Fixture::new();
    let alias = f.root.join("alias");
    symlink(&f.root, &alias).unwrap();
    assert!(Config::load(&alias.join("config/setup.json")).is_err());
    for (key, name) in [("auth_directory", "auth"), ("audit_directory", "audit")] {
        let mut config = f.value.clone();
        config[key] = json!(alias.join(name));
        f.rejects(&config);
    }
}

#[test]
fn unrelated_siblings_are_not_scanned_or_repaired_by_config_loading() {
    let f = Fixture::new();
    let sibling = f.root.join("config/unrelated.txt");
    fs::write(&sibling, b"unrelated fixture").unwrap();
    fs::set_permissions(&sibling, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Config::load(&f.path).is_ok());
    assert_eq!(fs::read(&sibling).unwrap(), b"unrelated fixture");
    assert_eq!(fs::metadata(&sibling).unwrap().mode() & 0o7777, 0o644);
    for name in ["auth", "audit"] {
        assert_eq!(fs::read_dir(f.root.join(name)).unwrap().count(), 0);
    }
}
