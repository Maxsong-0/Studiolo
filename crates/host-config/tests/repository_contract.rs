//! 私有配置批准生产仓库；只用本次 /tmp 合成目录，不打开任何数据库或源码。
#[path = "repository/drift.rs"]
mod drift;
#[path = "repository/fixture.rs"]
mod fixture;
#[path = "repository/layout.rs"]
mod layout;

use fixture::Fixture;
use serde_json::{Value, json};
use std::fs;
use studiolo_host_config::Config;

#[test]
fn absent_binding_does_not_even_require_repository_or_other_serving_paths() {
    let mut f = Fixture::new();
    f.document
        .as_object_mut()
        .unwrap()
        .remove("project_repository");
    f.write();
    let missing = f.root.join("must-not-open");
    assert!(
        Config::load(&f.config)
            .unwrap()
            .repository_serving_roots(&missing, &missing, Some(&missing), Some(&missing))
            .unwrap()
            .is_none()
    );
    assert!(!missing.exists());
}

#[test]
fn explicit_single_repository_defaults_to_directories_and_preserves_all_files() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let original = fs::read(&f.config).unwrap();
    let approved = f.approve().unwrap();
    assert_eq!(approved.project(), "项目/虚构工坊");
    assert_eq!(approved.directory(), f.repository.as_path());
    let metadata = fs::metadata(&f.repository).unwrap();
    assert_eq!(approved.root_identity(), (metadata.dev(), metadata.ino()));
    assert!(!approved.cargo_dependencies());
    approved.revalidate().unwrap();
    assert_eq!(fs::read(&f.config).unwrap(), original);
    for path in [
        &f.repository,
        &f.vault,
        &f.assets,
        &f.writer,
        &f.ingress,
        &f.root.join("auth"),
        &f.root.join("audit"),
    ] {
        assert_eq!(fs::read_dir(path).unwrap().count(), 0);
    }
}

#[test]
fn cargo_requires_explicit_boolean_and_rust_or_environment_paths_are_not_accepted() {
    let mut f = Fixture::new();
    f.document["project_repository"]["cargo_dependencies"] = json!(true);
    f.write();
    assert!(f.approve().unwrap().cargo_dependencies());
    for (key, value) in [
        ("cargo_dependencies", json!("true")),
        ("cargo_dependencies", Value::Null),
        ("rust_call_sites", json!(["src/lib.rs"])),
        ("seed_directory", json!(f.root.join("config"))),
    ] {
        let mut document = f.document.clone();
        document["project_repository"][key] = value;
        f.write_document(&document);
        assert!(Config::load(&f.config).is_err());
    }
}

#[test]
fn malformed_scope_wrong_project_and_ambiguous_json_fail_before_approval() {
    let f = Fixture::new();
    let scope = f.document["project_repository"].clone();
    for value in [
        Value::Null,
        json!([]),
        json!([1, "项目/虚构工坊", f.repository]),
        json!("repository"),
    ] {
        let mut document = f.document.clone();
        document["project_repository"] = value;
        f.write_document(&document);
        assert!(Config::load(&f.config).is_err());
    }
    for (key, value) in [
        ("version", json!(2)),
        ("directory", json!("relative")),
        ("directory", json!("/tmp/../private")),
        ("project", json!("项目")),
        ("project", json!("项目/虚构/src")),
        ("project", json!("项目/.hidden")),
        ("project", json!("sources/虚构")),
        ("project", json!("项目/../私有")),
    ] {
        let mut document = f.document.clone();
        document["project_repository"] = scope.clone();
        document["project_repository"][key] = value;
        f.write_document(&document);
        assert!(Config::load(&f.config).is_err());
    }
    let encoded = serde_json::to_string(&f.document).unwrap();
    let duplicate = encoded.replacen(
        "\"project_repository\":{",
        "\"project_repository\":{\"version\":1,",
        1,
    );
    fs::write(&f.config, duplicate).unwrap();
    assert!(Config::load(&f.config).is_err());
}
