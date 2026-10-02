use crate::support::{AuthorityViolation, rust_sources, source_violations, workspace_root};
use std::{fs, path::Path};

#[test]
fn bridge_private_storage_cannot_open_another_file_or_database() {
    let root = workspace_root();
    let storage = Path::new("crates/sync-bridge/src/sqlite/storage.rs");
    let source = fs::read_to_string(root.join(storage)).unwrap();
    assert!(source_violations("studiolo-sync-bridge", storage, &source).is_empty());
    for extra in [
        "std::fs::write(other, body)?;",
        "openat(&root, other, flags, Mode::empty())?;",
        "File::create(other)?;",
        "file.write_all(body)?;",
        "Command::new(program);",
        "openat(root, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())?;",
        "statat(root, name, AtFlags::SYMLINK_NOFOLLOW)?;",
        "rustix::fs::fstat(pin)?;",
    ] {
        assert!(
            source_violations(
                "studiolo-sync-bridge",
                storage,
                &format!("{source}\n{extra}")
            )
            .contains(&AuthorityViolation::GenericWrite),
            "固定 BridgeState 存储不能顺带取得其他写能力：{extra}"
        );
    }
    let follows_links = source.replace(
        "statat(root, name, AtFlags::SYMLINK_NOFOLLOW)",
        "statat(root, name, AtFlags::empty())",
    );
    assert_ne!(follows_links, source);
    assert!(
        source_violations("studiolo-sync-bridge", storage, &follows_links)
            .contains(&AuthorityViolation::GenericWrite)
    );
    for path in [
        "crates/sync-bridge/src/sqlite/opening.rs",
        "crates/sync-bridge/src/sqlite/database.rs",
        "crates/sync-bridge/src/planner.rs",
    ] {
        let path = Path::new(path);
        let source = fs::read_to_string(root.join(path)).unwrap();
        assert!(
            source_violations(
                "studiolo-sync-bridge",
                path,
                &format!("{source}\nConnection::open(other)?;")
            )
            .contains(&AuthorityViolation::DatabaseOpen)
        );
    }
}

#[test]
fn bridge_planner_stays_pure_and_sqlite_never_exports_its_connection() {
    let root = workspace_root();
    for path in rust_sources(&root.join("crates/sync-bridge/src")) {
        let source = fs::read_to_string(&path).unwrap();
        let compact = source.split_whitespace().collect::<String>();
        if path.components().any(|c| c.as_os_str() == "sqlite") {
            for forbidden in ["pubconnection:", "pubfnconnection("] {
                assert!(!compact.contains(forbidden), "{}", path.display());
            }
            continue;
        }
        for forbidden in [
            "rusqlite",
            "rustix",
            "getrandom",
            "studiolo_audit_ledger",
            "fs::",
            "net::",
            "process::",
            "sqlite::",
        ] {
            assert!(
                !compact.contains(forbidden),
                "纯规划器不能使用存储或运行权限：{} {forbidden}",
                path.display()
            );
        }
    }
}
