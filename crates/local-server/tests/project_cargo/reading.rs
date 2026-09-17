//! 固定根的真实文件读取与两轮间确定性变化；测试接缝不进入公开 API。

use super::test_support as support;

use std::fs;
use studiolo_core::{Revision, VaultUuid};

use super::observe_with;
use support::{API, Fixture, ROOT, empty};

#[test]
fn only_the_planned_manifests_are_read_twice_and_root_is_revalidated() {
    let f = Fixture::new();
    for ignored in ["Cargo.lock", "build.rs", "source.rs"] {
        f.write(ignored, "SOURCE_MUST_NOT_BE_READ");
    }
    fs::create_dir(f.root.path().join(".cargo")).unwrap();
    f.write(".cargo/config.toml", "SECRET_CONFIG_MUST_NOT_BE_READ");
    let reader = f.reader();
    let repository = f.repository();
    let mut reads = Vec::new();
    let mut checks = 0;
    let result = observe_with(
        |path, limit| {
            assert!(matches!(
                path.as_str(),
                "Cargo.toml" | "crates/api/Cargo.toml" | "crates/model/Cargo.toml"
            ));
            assert!(limit <= 64 * 1024);
            reads.push(path.to_string());
            reader.read_file(path, limit)
        },
        || {
            checks += 1;
            repository.revalidate()
        },
    );
    assert_eq!(result.status, "ready");
    assert_eq!(checks, 2);
    assert_eq!(
        reads,
        [
            "Cargo.toml",
            "crates/api/Cargo.toml",
            "crates/model/Cargo.toml"
        ]
        .repeat(2)
    );
    assert_eq!(result.graph.as_ref().unwrap().packages.len(), 2);
    assert_eq!(result.graph.as_ref().unwrap().dependencies.len(), 1);
    assert!(result.observed_at.as_ref().unwrap().parse::<u64>().unwrap() > 0);
    assert!(result.diagnostics.is_empty());
    assert!(!f.root.path().join(".workbench").exists());
    assert!(!f.root.path().join("target").exists());
}

#[test]
fn disabled_capability_returns_before_root_revalidation_or_manifest_reads() {
    let f = Fixture::new();
    let repository = f.repository();
    f.write("Cargo.toml", "MALFORMED_SECRET_INPUT");
    let moved = f.outside.path().join("retained");
    fs::rename(f.root.path(), &moved).unwrap();
    fs::create_dir(f.root.path()).unwrap();
    assert!(repository.revalidate().is_err());
    empty(&repository.observe_cargo_dependencies(), "disabled");
}

#[test]
fn the_complete_plan_is_checked_before_any_member_file_is_read() {
    for content in [
        "[workspace]\nmembers = [\"crates/api\", \"../outside\"]\n",
        "[workspace]\nmembers = [\"crates/api\", \"crates/api\"]\n",
        "[workspace]\nmembers = [\"crates/*\"]\n",
    ] {
        let f = Fixture::new();
        f.write("Cargo.toml", content);
        let reader = f.reader();
        let mut reads = Vec::new();
        let result = observe_with(
            |path, limit| {
                reads.push(path.to_string());
                reader.read_file(path, limit)
            },
            || Ok(()),
        );
        assert!(matches!(result.status, "invalid" | "unsupported"));
        assert!(result.graph.is_none());
        assert_eq!(reads, ["Cargo.toml"]);
    }
}

#[test]
fn changing_a_member_between_rounds_never_returns_the_first_graph() {
    for body in [
        API.replace("\"api\"", "\"changed-api\"").into_bytes(),
        vec![0xff],
    ] {
        let f = Fixture::new();
        let reader = f.reader();
        let mut reads = 0;
        let result = observe_with(
            |path, limit| {
                reads += 1;
                if reads == 4 {
                    fs::write(f.root.path().join("crates/api/Cargo.toml"), &body).unwrap();
                }
                reader.read_file(path, limit)
            },
            || Ok(()),
        );
        empty(&result, "unavailable");
        assert!((5..=6).contains(&reads));
    }
}

#[test]
fn changing_the_root_plan_does_not_authorize_reading_new_members_in_round_two() {
    let f = Fixture::new();
    let reader = f.reader();
    let mut reads = Vec::new();
    let result = observe_with(
        |path, limit| {
            reads.push(path.to_string());
            if reads.len() == 4 {
                f.write("Cargo.toml", "[workspace]\nmembers = [\"new-member\"]\n");
            }
            reader.read_file(path, limit)
        },
        || Ok(()),
    );
    empty(&result, "unavailable");
    assert_eq!(reads.len(), 4);
    assert!(!reads.iter().any(|path| path.contains("new-member")));
}

#[test]
fn root_replacement_after_both_reads_invalidates_the_observation() {
    let f = Fixture::new();
    let reader = f.reader();
    let repository = f.repository();
    let mut checks = 0;
    let result = observe_with(
        |path, limit| reader.read_file(path, limit),
        || {
            checks += 1;
            if checks == 2 {
                fs::rename(f.root.path(), f.outside.path().join("old-root")).unwrap();
                fs::create_dir(f.root.path()).unwrap();
                f.write("Cargo.toml", ROOT);
            }
            repository.revalidate()
        },
    );
    empty(&result, "unavailable");
}

#[test]
fn revision_and_body_are_both_required_to_match_between_rounds() {
    for change_revision in [true, false] {
        let f = Fixture::new();
        let reader = f.reader();
        let mut reads = 0;
        let result = observe_with(
            |path, limit| {
                reads += 1;
                let mut note = reader.read_file(path, limit)?;
                if reads == 5 {
                    if change_revision {
                        let namespace: VaultUuid =
                            "d67f1358-fd5f-43ca-82a4-704805a83ed2".parse().unwrap();
                        note.revision = Revision::compute(namespace, path, b"changed fingerprint");
                    } else {
                        note.content.extend_from_slice(b"\n# changed body\n");
                    }
                }
                Ok(note)
            },
            || Ok(()),
        );
        empty(&result, "unavailable");
    }
}
