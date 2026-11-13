use std::path::Path;

use crate::support::{AuthorityViolation, source_violations};

const STORAGE: &str = "crates/supply/src/storage.rs";
const CONNECTION: &str = "crates/supply/src/database.rs";

/// M3 供给面私有库 `agent.sqlite3` 与审计/Auth 私有存储同一形态：只创建固定文件名、
/// 不获得 canonical 能力；库只能由 database.rs 的唯一 `open_with_flags` 打开。
#[test]
fn supply_private_storage_is_fixed_and_separate_from_audit_and_auth() {
    let fixture = r#"
        use std::{fs::{self, OpenOptions}, os::unix::fs::OpenOptionsExt};
        let metadata = fs::symlink_metadata(root)?;
        let database = root.join("agent.sqlite3");
        let file = OpenOptions::new().read(true).write(true).create_new(true)
            .mode(0o600).open(&database)?;
        file.sync_all()?;
        fs::File::open(root).and_then(|file| file.sync_all())?;
    "#;
    assert!(source_violations("studiolo-supply", Path::new(STORAGE), fixture).is_empty());
    for changed in [
        fixture.replace("agent.sqlite3", "audit.sqlite3"),
        fixture.replace("agent.sqlite3", "auth.sqlite3"),
        fixture.replace("agent.sqlite3", "notes.md"),
        format!("{fixture}\nlet database = root.join(\"notes.md\");"),
        format!("{fixture}\nfs::write(path, bytes)?;"),
    ] {
        assert!(
            source_violations("studiolo-supply", Path::new(STORAGE), &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for (name, path) in [
        ("studiolo-agent-server", STORAGE),
        ("studiolo-supply", CONNECTION),
        ("studiolo-supply", "crates/supply/src/roots.rs"),
    ] {
        assert!(
            source_violations(name, Path::new(path), fixture)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    let source = "let conn = Connection::open_with_flags(&storage.database, flags)?;";
    assert!(source_violations("studiolo-supply", Path::new(CONNECTION), source).is_empty());
    assert!(
        source_violations("studiolo-supply", Path::new(STORAGE), source)
            .contains(&AuthorityViolation::DatabaseOpen)
    );
    assert!(
        source_violations(
            "studiolo-supply",
            Path::new(CONNECTION),
            &format!("{source}\n{source}")
        )
        .contains(&AuthorityViolation::DatabaseOpen)
    );
}

#[test]
fn derived_index_storage_has_its_own_fixed_file_and_connection() {
    let path = Path::new("crates/supply/src/index_cache/paths.rs");
    let connection = Path::new("crates/supply/src/index_cache/mod.rs");
    let fixture = r#"
        use std::{fs::{self, OpenOptions}, os::unix::fs::OpenOptionsExt};
        let canonical = fs::canonicalize(root)?;
        let metadata = fs::symlink_metadata(root)?;
        let database = root.join("index.sqlite3");
        let file = OpenOptions::new().read(true).write(true).create_new(true)
            .mode(0o600).open(&database)?;
        file.sync_all()?;
        fs::File::open(root).and_then(|file| file.sync_all())?;
    "#;
    assert!(source_violations("studiolo-supply", path, fixture).is_empty());
    for changed in [
        fixture.replace("index.sqlite3", "agent.sqlite3"),
        fixture.replace("index.sqlite3", "audit.sqlite3"),
        fixture.replace("index.sqlite3", "document.md"),
        format!("{fixture}\nfs::write(path, bytes)?;"),
    ] {
        assert!(
            source_violations("studiolo-supply", path, &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for other in [Path::new(STORAGE), connection] {
        assert!(
            source_violations("studiolo-supply", other, fixture)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    let sql = "let conn = Connection::open_with_flags(&storage.database, flags)?;";
    assert!(source_violations("studiolo-supply", connection, sql).is_empty());
    assert!(
        source_violations("studiolo-supply", path, sql).contains(&AuthorityViolation::DatabaseOpen)
    );
    assert!(
        source_violations(
            "studiolo-supply",
            connection,
            "Connection::open(root.join(\"agent.sqlite3\"))"
        )
        .contains(&AuthorityViolation::DatabaseOpen)
    );
}

#[test]
fn identity_writers_cannot_enter_human_or_other_production_crates() {
    for symbol in [
        "AgentStore",
        "register_instance",
        "register_child_instance",
        "touch_instance",
        "issue_type",
        "revoke_type",
        "record_read",
        "rebuild_reads",
        "upgrade_child_instances",
    ] {
        for name in [
            "studiolo-local-server",
            "studiolo-workbench",
            "studiolo-gateway",
        ] {
            assert!(
                source_violations(
                    name,
                    Path::new("src/lib.rs"),
                    &format!("let x = {symbol}::open(value); {symbol}(value);")
                )
                .contains(&AuthorityViolation::IdentityWrite)
            );
        }
    }
    for name in ["studiolo-supply", "studiolo-agent-server"] {
        assert!(
            source_violations(
                name,
                Path::new("src/lib.rs"),
                "AgentStore::open_existing(path)"
            )
            .is_empty()
        );
    }
    assert!(
        source_violations(
            "studiolo-local-server",
            Path::new("src/lib.rs"),
            "AgentIdentityView::open_existing(path); PolicyAgentStore::open_existing(policy)"
        )
        .is_empty()
    );
}

#[test]
fn identity_view_guard_rejects_import_alias_but_accepts_backup_kind() {
    for source in [
        "use studiolo_supply::{AgentStore as Writer};",
        "use studiolo_supply::AgentStore;",
        "let s = studiolo_supply::AgentStore ::open(path);",
    ] {
        assert!(
            source_violations("studiolo-local-server", Path::new("src/lib.rs"), source)
                .contains(&AuthorityViolation::IdentityWrite)
        );
    }
    assert!(
        source_violations(
            "studiolo-core",
            Path::new("src/lib.rs"),
            "enum CheckpointKind { AgentStore } let kind = CheckpointKind::AgentStore;"
        )
        .is_empty()
    );
}
