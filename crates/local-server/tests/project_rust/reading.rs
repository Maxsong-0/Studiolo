//! 固定白名单真实读取；私有接缝提供确定性两轮变化，不暴露到公共 API。

use std::fs;
use studiolo_core::{Revision, VaultPath, VaultUuid};

use super::{
    observe_with,
    test_support::{API, Fixture, empty, paths, ready},
};

#[test]
fn exactly_the_sorted_whitelist_is_read_twice_without_discovery_or_writes() {
    let f = Fixture::new();
    for ignored in [
        "src/hidden_module.rs",
        "src/outside.rs",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
    ] {
        f.write(ignored, "UNREAD_PRIVATE_CONTENT");
    }
    let reader = f.reader();
    let repository = f.repository();
    let mut requested = paths();
    requested.reverse();
    let mut reads = Vec::new();
    let mut checks = 0;
    let result = observe_with(
        &requested,
        |path, limit| {
            assert!(paths().iter().any(|allowed| allowed == path.as_str()));
            assert!(limit <= 64 * 1024);
            reads.push(path.to_string());
            reader.read_file(path, limit)
        },
        || {
            checks += 1;
            repository.revalidate()
        },
    );
    ready(&result);
    assert_eq!(reads, [paths(), paths()].concat());
    assert_eq!(checks, 2);
    let graph = result.graph.as_ref().unwrap();
    assert_eq!(graph.files.len(), 2);
    assert_eq!(graph.calls.len(), 2);
    assert_eq!(graph.files[0].external_modules, 1);
    assert_eq!(graph.files[0].unexpanded_macros, 1);
    assert!(result.observed_at.as_ref().unwrap().parse::<u64>().unwrap() > 0);
    let encoded = serde_json::to_string(&result).unwrap();
    for secret in [
        "ARGUMENT_SECRET",
        "UNREAD_PRIVATE_CONTENT",
        f.root.path().to_str().unwrap(),
    ] {
        assert!(!encoded.contains(secret));
    }
    assert!(!f.root.path().join("target").exists());
    assert!(!f.root.path().join(".workbench").exists());
}

#[test]
fn disabled_returns_before_root_io_and_enabling_only_checks_paths() {
    let f = Fixture::new();
    let repository = f.repository().with_cargo_dependencies();
    fs::rename(f.root.path(), f.outside.path().join("retained-root")).unwrap();
    fs::create_dir(f.root.path()).unwrap();
    empty(&repository.observe_rust_call_sites(), "disabled");
    // 根已替换且文件全缺失，能力装配仍只做词法检查。
    let enabled = repository.with_rust_call_sites(paths()).unwrap();
    empty(&enabled.observe_rust_call_sites(), "unavailable");
}

#[test]
fn invalid_whole_whitelists_are_rejected_before_root_or_source_io() {
    for paths in [
        vec![],
        vec!["src/api.rs", "../secret.rs"],
        vec!["src/api.rs", "src/api.rs"],
        vec!["Src/api.rs", "src/model.rs"],
        vec!["src/api.rs", ".hidden.rs"],
        vec!["src/api.rs", "src%2Fsecret.rs"],
        vec!["src/api.rs", "src/a\u{200b}.rs"],
        vec!["src/api.rs", "src/./model.rs"],
    ] {
        let paths = paths.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let result = observe_with(
            &paths,
            |_, _| panic!("不应读取源码"),
            || panic!("不应复核根"),
        );
        empty(&result, "invalid");
        assert!(
            Fixture::new()
                .repository()
                .with_rust_call_sites(paths)
                .is_err()
        );
    }
}

#[test]
fn changed_body_revision_and_second_round_utf8_are_unavailable() {
    for change in 0..3 {
        let f = Fixture::new();
        let reader = f.reader();
        let mut reads = 0;
        let result = observe_with(
            &paths(),
            |path, limit| {
                reads += 1;
                let mut note = reader.read_file(path, limit)?;
                if reads == 3 {
                    match change {
                        0 => {
                            let changed = API.replace("helper", "worker").into_bytes();
                            assert_eq!(changed.len(), note.content.len());
                            note.content = changed;
                        }
                        1 => {
                            let namespace: VaultUuid =
                                "d67f1358-fd5f-43ca-82a4-704805a83ed2".parse().unwrap();
                            note.revision = Revision::compute(namespace, path, b"changed revision");
                        }
                        _ => note.content = vec![0xff],
                    }
                }
                Ok(note)
            },
            || Ok(()),
        );
        empty(&result, "unavailable");
    }
}

#[test]
fn filesystem_file_replacement_between_rounds_rejects_the_batch() {
    let f = Fixture::new();
    let reader = f.reader();
    let mut reads = 0;
    let result = observe_with(
        &paths(),
        |path, limit| {
            reads += 1;
            if reads == 3 {
                f.write("src/replacement.rs", "fn changed() {}\n");
                fs::rename(
                    f.root.path().join("src/replacement.rs"),
                    f.root.path().join("src/api.rs"),
                )
                .unwrap();
            }
            reader.read_file(path, limit)
        },
        || Ok(()),
    );
    empty(&result, "unavailable");
}

#[test]
fn final_root_replacement_and_mismatched_read_paths_cannot_publish() {
    let f = Fixture::new();
    let reader = f.reader();
    let repository = f.repository();
    let mut checks = 0;
    let result = observe_with(
        &paths(),
        |path, limit| reader.read_file(path, limit),
        || {
            checks += 1;
            if checks == 2 {
                fs::rename(f.root.path(), f.outside.path().join("old-root")).unwrap();
                fs::create_dir(f.root.path()).unwrap();
            }
            repository.revalidate()
        },
    );
    empty(&result, "unavailable");
    let f = Fixture::new();
    let reader = f.reader();
    let result = observe_with(
        &paths(),
        |path, limit| {
            let mut note = reader.read_file(path, limit)?;
            note.path = VaultPath::parse("other.rs").unwrap();
            Ok(note)
        },
        || Ok(()),
    );
    empty(&result, "unavailable");
}

#[test]
fn empty_unicode_files_and_controlled_parse_failures_have_distinct_results() {
    let f = Fixture::new();
    f.write("src/中文.rs", "");
    let result = f
        .repository()
        .with_rust_call_sites(vec!["src/中文.rs".into()])
        .unwrap()
        .observe_rust_call_sites();
    ready(&result);
    let graph = result.graph.unwrap();
    assert_eq!(graph.files[0].path, "src/中文.rs");
    assert!(graph.calls.is_empty());
    for (body, status) in [
        ("fn SECRET_SYNTAX( {".to_owned(), "invalid"),
        (
            format!("fn deep() {{ {}0{}; }}", "(".repeat(80), ")".repeat(80)),
            "unsupported",
        ),
    ] {
        f.write("src/api.rs", &body);
        let result = f.observe();
        empty(&result, status);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("SECRET_SYNTAX")
        );
    }
    f.write("src/api.rs", API);
}
