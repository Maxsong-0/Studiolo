//! 新增的私有适配器沿用原有的词法权限边界。
use crate::support::{AuthorityViolation, source_violations, workspace_root};
use std::{fs, path::Path};
const SITES: &[(&str, &str)] = &[
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/cleanup.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/real_tests.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/landing_store.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/landing_candidate.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/socket_tests.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/landing_tests.rs",
    ),
    ("studiolo", "crates/studiolo/src/demo_gateway.rs"),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/roots.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/lifecycle.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/isolation.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/real.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/snapshot.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/landing.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/git_trust.rs",
    ),
    (
        "studiolo-runner-adapter",
        "crates/runner-adapter/src/git_trust/audit.rs",
    ),
    ("studiolo-gateway", "crates/gateway/src/relay_store.rs"),
    ("studiolo-gateway", "crates/gateway/src/durable_relay.rs"),
    ("studiolo-gateway", "crates/gateway/src/relay_retention.rs"),
    (
        "studiolo-gateway",
        "crates/gateway/src/bin/gateway-model-relay.rs",
    ),
    ("studiolo-gateway", "crates/gateway/src/relay_remote.rs"),
    ("studiolo-gateway", "crates/gateway/src/relay_unix.rs"),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/storage.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/startup/runner_input.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/config.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/socket.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/start.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/publish.rs",
    ),
    (
        "studiolo-local-server",
        "crates/local-server/src/board/executor/recovery.rs",
    ),
];
#[test]
fn m5_m6_authority_exceptions_are_exact_and_do_not_swallow_new_calls() {
    for (crate_name, name) in SITES {
        let path = Path::new(name);
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        assert!(
            source_violations(crate_name, path, &source).is_empty(),
            "{name}"
        );
        for extra in [
            "std::fs::write(other, bytes);",
            "fs::read(other);",
            "Command::new(\"sh\").arg(\"-c\").arg(script);",
            "File::options().write(true).open(other);",
            "file.write_all(arbitrary);",
        ] {
            assert!(
                source_violations(crate_name, path, &format!("{source}\n{extra}"))
                    .contains(&AuthorityViolation::GenericWrite),
                "{name}: {extra}"
            );
        }
        assert!(
            source_violations(
                crate_name,
                Path::new("crates/unregistered/src/adapter.rs"),
                &source
            )
            .contains(&AuthorityViolation::GenericWrite),
            "misplaced {name}"
        );
        assert!(
            source_violations("studiolo-task-engine", path, &source)
                .contains(&AuthorityViolation::GenericWrite),
            "wrong crate {name}"
        );
    }
}
#[test]
fn fixed_targets_and_registered_call_counts_cannot_be_changed() {
    for (crate_name, name, before, after) in [
        (
            "studiolo-local-server",
            "crates/local-server/src/board/storage.rs",
            "control.sqlite3",
            "notes.md",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "Mode::RUSR | Mode::WUSR",
            "Mode::RUSR | Mode::WUSR | Mode::WOTH",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/isolation.rs",
            "Command::new(&self.systemd_run)",
            "Command::new(other)",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            "fs::remove_file(path)",
            "fs::remove_file(other)",
        ),
        (
            "studiolo-gateway",
            "crates/gateway/src/bin/gateway-model-relay.rs",
            "0o660",
            "0o666",
        ),
        (
            "studiolo-gateway",
            "crates/gateway/src/relay_store.rs",
            ".open(path)",
            ".open(other)",
        ),
    ] {
        let path = Path::new(name);
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        let changed = source.replace(before, after);
        assert_ne!(source, changed, "fixture anchor: {name}");
        assert!(
            source_violations(crate_name, path, &changed)
                .contains(&AuthorityViolation::GenericWrite),
            "{name}"
        );
    }
    for (crate_name, name, extra) in [
        (
            "studiolo-local-server",
            "crates/local-server/src/board/storage.rs",
            "fs::symlink_metadata(directory);",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/isolation.rs",
            "Command::new(&self.systemd_run);",
        ),
        (
            "studiolo-gateway",
            "crates/gateway/src/relay_retention.rs",
            "fs::read_dir(root);",
        ),
    ] {
        let path = Path::new(name);
        let source = fs::read_to_string(workspace_root().join(path)).unwrap();
        assert!(
            source_violations(crate_name, path, &format!("{source}\n{extra}"))
                .contains(&AuthorityViolation::GenericWrite),
            "{name}"
        );
    }
}
#[test]
fn gateway_sqlite_is_one_registered_opening_and_not_a_crate_wide_exception() {
    let name = "crates/gateway/src/relay_store.rs";
    let path = Path::new(name);
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    for (path, changed) in [
        (path, format!("{source}\nConnection::open(path);")),
        (
            path,
            source.replace("Connection::open(path)", "Connection::open(other)"),
        ),
        (
            Path::new("crates/gateway/src/other.rs"),
            "Connection::open(path);".into(),
        ),
    ] {
        assert!(
            source_violations("studiolo-gateway", path, &changed)
                .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
}
#[test]
fn m0_result_exception_remains_limited_to_one_temp_result() {
    let source = r#"let result_path=temp_dir.path().join("result.md");
        let mut file=File::create(&result_path)?;file.write_all(bytes)?;file.sync_all()?;"#;
    let path = Path::new("crates/runner-adapter/src/lib.rs");
    assert!(source_violations("studiolo-runner-adapter", path, source).is_empty());
    assert!(
        source_violations(
            "studiolo-runner-adapter",
            path,
            &format!("{source} file.write_all(extra)?;")
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
    assert!(
        source_violations(
            "studiolo-runner-adapter",
            Path::new("crates/runner-adapter/src/extra.rs"),
            source
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
}

#[path = "m6_final_authority.rs"]
mod final_authority;
