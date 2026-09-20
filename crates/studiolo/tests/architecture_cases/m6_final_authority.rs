//! Final M6 authority exceptions remain closed under extra calls, weaker guards and relocation.
use crate::support::{AuthorityViolation, source_violations, workspace_root};
use std::{fs, path::Path};

fn source(path: &str) -> String {
    fs::read_to_string(workspace_root().join(path)).unwrap()
}

#[test]
fn final_guards_and_fixed_targets_cannot_be_weakened() {
    for (package, path, before, after) in [
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "identity(&stat) != self.identity",
            "false",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "AtFlags::SYMLINK_NOFOLLOW",
            "AtFlags::empty()",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/landing_store.rs",
            "landing-{key}-{phase}.json",
            "arbitrary-{key}.json",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/landing_candidate.rs",
            "object.revision != pointer.revision",
            "false",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/cleanup.rs",
            "isolate_unit(&r.opaque_id)?;",
            "",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/cleanup.rs",
            "let mut protected = all.clone();",
            "let mut protected = BTreeSet::new();",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            ".studiolo-snapshot-cache.json",
            "arbitrary.json",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            "request.active_attempts.contains(&attempt)",
            "false",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            "snapshot.preserve(&destination)?",
            "unverified_candidate()",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/real.rs",
            "run.retained.with_extension(\"stopped.json\")",
            "arbitrary_path",
        ),
        (
            "studiolo",
            "crates/studiolo/src/demo_gateway.rs",
            "!config.endpoint.ip().is_loopback()",
            "false",
        ),
        (
            "studiolo",
            "crates/studiolo/src/demo_gateway.rs",
            "OpenFlags::SQLITE_OPEN_READ_ONLY",
            "OpenFlags::SQLITE_OPEN_READ_WRITE",
        ),
    ] {
        let original = source(path);
        let changed = original.replace(before, after);
        assert_ne!(original, changed, "missing negative fixture anchor: {path}");
        assert!(
            !source_violations(package, Path::new(path), &changed).is_empty(),
            "{path}: {before}"
        );
    }
}

#[test]
fn bare_rustix_and_socket_calls_cannot_piggyback_on_import_exceptions() {
    for (package, path, extra) in [
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "unlinkat(other, arbitrary, AtFlags::empty());",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "open(other, OFlags::RDWR, Mode::RUSR);",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/socket.rs",
            "UnixListener::bind(other);",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/landing_store.rs",
            "open(other, OFlags::RDWR, Mode::RUSR);",
        ),
        (
            "studiolo-local-server",
            "crates/local-server/src/board/executor/cleanup.rs",
            "fs::read_dir(&profile.attempt_root);",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            "File::open(other);",
        ),
        (
            "studiolo-runner-adapter",
            "crates/runner-adapter/src/snapshot.rs",
            "temporary.persist(other);",
        ),
        (
            "studiolo",
            "crates/studiolo/src/demo_gateway.rs",
            "TcpStream::connect_timeout(&other, timeout);",
        ),
    ] {
        let injected = format!("{}\n{extra}", source(path));
        assert!(
            source_violations(package, Path::new(path), &injected)
                .contains(&AuthorityViolation::GenericWrite),
            "{path}: {extra}"
        );
    }
}

#[test]
fn demo_sqlite_and_canonical_candidate_read_have_no_ambient_exception() {
    let demo = "crates/studiolo/src/demo_gateway.rs";
    for (path, value) in [
        (demo, format!("{}\nConnection::open(other);", source(demo))),
        (
            "crates/studiolo/src/other.rs",
            "Connection::open(other);".into(),
        ),
    ] {
        assert!(
            source_violations("studiolo", Path::new(path), &value)
                .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
    assert!(
        source_violations(
            "studiolo",
            Path::new("crates/studiolo/src/other.rs"),
            "TcpStream::connect_timeout(&other, timeout);"
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
    let candidate = "crates/local-server/src/board/executor/landing_candidate.rs";
    let extra = format!(
        "{}\nlet forged = \"sources/attempts/arbitrary\";",
        source(candidate)
    );
    assert!(
        source_violations("studiolo-local-server", Path::new(candidate), &extra)
            .contains(&AuthorityViolation::CanonicalPath)
    );
}

#[test]
fn executor_module_lost_its_old_socket_filesystem_authority() {
    let path = "crates/local-server/src/board/executor/mod.rs";
    let original = source(path);
    assert!(source_violations("studiolo-local-server", Path::new(path), &original).is_empty());
    for extra in [
        "std::fs::remove_file(&self.socket);",
        "std::fs::set_permissions(&socket, mode);",
        "Command::new(\"systemctl\");",
    ] {
        assert!(
            source_violations(
                "studiolo-local-server",
                Path::new(path),
                &format!("{original}\n{extra}")
            )
            .contains(&AuthorityViolation::GenericWrite)
        );
    }
}
