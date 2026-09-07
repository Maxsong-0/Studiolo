use std::path::Path;

use crate::support::{AuthorityViolation, source_violations};

const STORAGE: &str = "crates/audit-ledger/src/sqlite/storage.rs";

#[test]
fn audit_filesystem_allowance_is_exact_and_does_not_grant_vault_authority() {
    let fixture = r#"
        use std::{fs::{self, OpenOptions}, os::unix::fs::OpenOptionsExt};
        let metadata = fs::symlink_metadata(root)?;
        let database = root.join("audit.sqlite3");
        let file = OpenOptions::new().read(true).write(true).create_new(true)
            .mode(0o600).open(&database)?;
        file.sync_all()?;
        fs::File::open(root).and_then(|file| file.sync_all())?;
    "#;
    assert!(source_violations("studiolo-audit-ledger", Path::new(STORAGE), fixture).is_empty());
    for changed in [
        fixture.replace("audit.sqlite3", "notes.md"),
        format!("{fixture}\nlet database = root.join(\"notes.md\");"),
        format!("{fixture}\nOpenOptions::new().read(true).open(other)?;"),
    ] {
        assert!(
            source_violations("studiolo-audit-ledger", Path::new(STORAGE), &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for (crate_name, path) in [
        ("studiolo-gateway", STORAGE),
        ("studiolo-audit-ledger", "crates/audit-ledger/src/memory.rs"),
        ("studiolo-audit-ledger", "crates/audit-ledger/src/sqlite.rs"),
    ] {
        assert!(
            source_violations(crate_name, Path::new(path), fixture)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for forbidden in [
        "std::fs::write(path, bytes)?;",
        "fs::remove_file(path)?;",
        "OpenOptions::new().write(true).truncate(true).open(path)?;",
        "File::create(path)?;",
        "file.write_all(bytes)?;",
    ] {
        let source = format!("{fixture}\n{forbidden}");
        assert!(
            source_violations("studiolo-audit-ledger", Path::new(STORAGE), &source)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    let source = format!("{fixture}\nlet forbidden = \".workbench/vault.json\";");
    assert!(
        source_violations("studiolo-audit-ledger", Path::new(STORAGE), &source)
            .contains(&AuthorityViolation::CanonicalPath)
    );
    let source = format!("{fixture}\nlet root = cap_std::ambient_authority();");
    assert!(
        source_violations("studiolo-audit-ledger", Path::new(STORAGE), &source)
            .contains(&AuthorityViolation::AmbientRoot)
    );
}

#[test]
fn audit_database_can_only_be_opened_by_the_sqlite_adapter() {
    let source = "let conn = Connection::open_with_flags(&storage.database, flags)?;";
    assert!(
        source_violations(
            "studiolo-audit-ledger",
            Path::new("crates/audit-ledger/src/sqlite.rs"),
            source
        )
        .is_empty()
    );
    assert!(
        source_violations("studiolo-audit-ledger", Path::new(STORAGE), source)
            .contains(&AuthorityViolation::DatabaseOpen)
    );
    for changed in [
        source.replace("&storage.database", "path"),
        format!("{source}\n{source}"),
    ] {
        assert!(
            source_violations(
                "studiolo-audit-ledger",
                Path::new("crates/audit-ledger/src/sqlite.rs"),
                &changed
            )
            .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
}
