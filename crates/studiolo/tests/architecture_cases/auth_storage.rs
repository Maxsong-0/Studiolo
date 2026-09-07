use std::path::Path;

use crate::support::{AuthorityViolation, source_violations, workspace_root};

const STORAGE: &str = "crates/auth/src/session/storage.rs";
const CONNECTION: &str = "crates/auth/src/session/database.rs";

#[test]
fn auth_storage_is_fixed_private_and_separate_from_audit() {
    let fixture = r#"
        use std::{fs::{self, OpenOptions}, os::unix::fs::OpenOptionsExt};
        let metadata = fs::symlink_metadata(root)?;
        let database = root.join("auth.sqlite3");
        let file = OpenOptions::new().read(true).write(true).create_new(true)
            .mode(0o600).open(&database)?;
        file.sync_all()?;
        fs::File::open(root).and_then(|file| file.sync_all())?;
    "#;
    assert!(source_violations("studiolo-auth", Path::new(STORAGE), fixture).is_empty());
    for changed in [
        fixture.replace("auth.sqlite3", "audit.sqlite3"),
        fixture.replace("auth.sqlite3", "notes.md"),
        format!("{fixture}\nlet database = root.join(\"notes.md\");"),
        format!("{fixture}\nOpenOptions::new().read(true).open(other)?;"),
        format!("{fixture}\nfs::write(path, bytes)?;"),
    ] {
        assert!(
            source_violations("studiolo-auth", Path::new(STORAGE), &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for (name, path) in [
        ("studiolo-audit-ledger", STORAGE),
        ("studiolo-local-server", STORAGE),
        ("studiolo-auth", CONNECTION),
        ("studiolo-auth", "crates/auth/src/session/commands.rs"),
    ] {
        assert!(
            source_violations(name, Path::new(path), fixture)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for (forbidden, violation) in [
        (".workbench/vault.json", AuthorityViolation::CanonicalPath),
        ("open_ambient_dir", AuthorityViolation::AmbientRoot),
    ] {
        assert!(
            source_violations(
                "studiolo-auth",
                Path::new(STORAGE),
                &format!("{fixture}{forbidden}")
            )
            .contains(&violation)
        );
    }
}

#[test]
fn auth_database_open_is_only_owned_by_its_adapter() {
    let source = "let conn = Connection::open_with_flags(&storage.database, flags)?;";
    assert!(source_violations("studiolo-auth", Path::new(CONNECTION), source).is_empty());
    for path in [
        STORAGE,
        "crates/auth/src/access/mod.rs",
        "crates/audit-ledger/src/sqlite.rs",
    ] {
        assert!(
            source_violations("studiolo-auth", Path::new(path), source)
                .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
    for changed in [
        source.replace("&storage.database", "path"),
        format!("{source}{source}"),
    ] {
        assert!(
            source_violations("studiolo-auth", Path::new(CONNECTION), &changed)
                .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
}

#[test]
fn deployment_and_ci_require_the_production_auth_feature() {
    let root = workspace_root();
    for path in [
        "scripts/package-preview.sh",
        "scripts/test-linux.sh",
        ".github/workflows/ci.yml",
    ] {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        assert!(
            source.contains("-p studiolo-local-server --features studiolo-auth/production"),
            "{path}"
        );
    }
    let source = std::fs::read_to_string(root.join("crates/auth/src/lib.rs")).unwrap();
    assert!(source.contains("#[cfg(all(feature = \"production\", feature = \"test-support\"))]"));
    assert!(
        source.contains("compile_error!(\"STUDIOLO_AUTH_TEST_FIXTURE_FORBIDDEN_IN_PRODUCTION\")")
    );
}
