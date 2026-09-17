use super::support::*;
use crate::sqlite::*;
use std::{fs, process::Command};

#[test]
fn existing_does_not_create_files_and_initialize_does_not_reuse() {
    let f = Fixture::new();
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::MissingStore)
    ));
    assert_eq!(fs::read_dir(&f.root).unwrap().count(), 0);
    let store = f.init();
    drop(store);
    assert!(matches!(
        BridgeState::initialize(&f.root, binding(), crate::Limits::conservative(1_000_000)),
        Err(BridgeError::AlreadyExists)
    ));
    assert_eq!(f.reopen().view().unwrap().version, 0);
}
#[test]
fn ownership_mode_and_pinned_file_replacements_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    for name in ["bridge.sqlite3", "bridge.lock"] {
        let f = Fixture::new();
        let store = f.init();
        fs::rename(f.root.join(name), f.root.join(format!("{name}.old"))).unwrap();
        fs::write(f.root.join(name), b"replacement").unwrap();
        fs::set_permissions(f.root.join(name), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(store.view(), Err(BridgeError::UnsafeStorage)));
    }
    let f = Fixture::new();
    let store = f.init();
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(matches!(store.view(), Err(BridgeError::UnsafeStorage)));
}
#[test]
fn symlink_hardlink_and_fifo_fail_without_following_or_waiting() {
    use std::os::unix::fs::symlink;
    for kind in 0..3 {
        let f = Fixture::new();
        drop(f.init());
        let name = f.root.join("bridge.lock");
        fs::remove_file(&name).unwrap();
        match kind {
            0 => symlink(f.root.join("bridge.sqlite3"), &name).unwrap(),
            1 => fs::hard_link(f.root.join("bridge.sqlite3"), &name).unwrap(),
            _ => assert!(
                std::process::Command::new("/usr/bin/mkfifo")
                    .arg(&name)
                    .status()
                    .unwrap()
                    .success()
            ),
        }
        assert!(matches!(
            BridgeState::open_existing(&f.root, binding()),
            Err(BridgeError::UnsafeStorage)
        ));
    }
}
#[test]
fn root_replacement_same_path_is_detected() {
    use std::os::unix::fs::DirBuilderExt;
    let f = Fixture::new();
    let store = f.init();
    let old = f.root.with_extension("old");
    fs::rename(&f.root, &old).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&f.root).unwrap();
    assert!(matches!(store.view(), Err(BridgeError::UnsafeStorage)));
    drop(store);
    fs::remove_dir_all(old).unwrap();
}
#[test]
fn second_process_cannot_acquire_owner_lease() {
    let f = Fixture::new();
    let _store = f.init();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sqlite::tests::storage_contract::lease_child",
            "--nocapture",
        ])
        .env("STUDIOLO_BRIDGE_DRAFT_LEASE_TEST", &f.root)
        .output()
        .unwrap();
    assert!(result.status.success(), "lease child contract failed");
}
#[test]
fn lease_child() {
    let Some(path) = std::env::var_os("STUDIOLO_BRIDGE_DRAFT_LEASE_TEST") else {
        return;
    };
    assert!(matches!(
        BridgeState::open_existing(std::path::Path::new(&path), binding()),
        Err(BridgeError::Busy)
    ));
}
#[test]
fn external_sqlite_write_invalidates_cached_plan_even_without_revision_change() {
    let f = Fixture::new();
    let store = f.init();
    let external = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    external
        .execute("UPDATE bridge_meta SET encoded_total=encoded_total", [])
        .unwrap();
    // 明确改变记录使 SQLite 产生提交；不依赖 no-op UPDATE 是否写页。
    external.execute_batch("PRAGMA user_version=2;").unwrap();
    assert!(matches!(store.view(), Err(BridgeError::VersionConflict)));
    drop(external);
    drop(store);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::UnsupportedSchema)
    ));
}
