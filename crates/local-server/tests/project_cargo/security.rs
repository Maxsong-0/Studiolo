//! 链接与文件替换沿用 Broker 根边界；所有拒绝均不泄漏正文或宿主路径。
#![cfg(unix)]

use super::test_support as support;

use std::{fs, os::unix::fs::symlink};

use support::{Fixture, empty};

#[test]
fn symlinked_manifests_and_parent_components_are_never_followed() {
    for target in ["Cargo.toml", "crates/api/Cargo.toml", "crates/api"] {
        let f = Fixture::new();
        let source = f.root.path().join(target);
        let outside = f.outside.path().join("secret");
        if source.is_dir() {
            fs::rename(&source, &outside).unwrap();
        } else {
            fs::write(&outside, "SOURCE_SECRET_MUST_NOT_APPEAR").unwrap();
            fs::remove_file(&source).unwrap();
        }
        symlink(&outside, &source).unwrap();
        let result = f.observe();
        empty(&result, "unavailable");
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("SOURCE_SECRET_MUST_NOT_APPEAR"));
        assert!(!encoded.contains(f.outside.path().to_str().unwrap()));
    }
}

#[test]
fn hard_links_special_nodes_and_case_collisions_cannot_form_manifest_evidence() {
    let f = Fixture::new();
    fs::hard_link(
        f.root.path().join("crates/api/Cargo.toml"),
        f.outside.path().join("linked.toml"),
    )
    .unwrap();
    empty(&f.observe(), "unavailable");
    fs::remove_file(f.outside.path().join("linked.toml")).unwrap();
    fs::remove_file(f.root.path().join("crates/api/Cargo.toml")).unwrap();
    fs::create_dir(f.root.path().join("crates/api/Cargo.toml")).unwrap();
    empty(&f.observe(), "unavailable");
    let f = Fixture::new();
    f.write("cargo.toml", "case collision");
    if fs::read_dir(f.root.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("cargo.toml")
        })
        .count()
        == 2
    {
        empty(&f.observe(), "unavailable");
    }
}

#[test]
fn missing_unreadable_or_non_utf8_manifests_never_publish_a_partial_graph() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::remove_file(f.root.path().join("crates/api/Cargo.toml")).unwrap();
    empty(&f.observe(), "unavailable");
    fs::write(f.root.path().join("crates/api/Cargo.toml"), [0xff, 0xfe]).unwrap();
    empty(&f.observe(), "invalid");
    if rustix::process::geteuid().as_raw() != 0 {
        fs::set_permissions(
            f.root.path().join("crates/api/Cargo.toml"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let result = f.observe();
        fs::set_permissions(
            f.root.path().join("crates/api/Cargo.toml"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        empty(&result, "unavailable");
    }
}

#[test]
fn malformed_or_unsupported_toml_diagnostics_are_controlled_text() {
    for (body, status) in [
        ("[package]\nname = \"SOURCE_SECRET\n", "invalid"),
        (
            "[package]\nname = \"api\"\n[workspace]\nmembers = []\n",
            "unsupported",
        ),
    ] {
        let f = Fixture::new();
        f.write("crates/api/Cargo.toml", body);
        let result = f.observe();
        empty(&result, status);
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("SOURCE_SECRET"));
        assert!(!encoded.contains(f.root.path().to_str().unwrap()));
    }
}
