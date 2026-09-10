//! 派生缓存契约：整批持久化、来源隔离、损坏拒绝与固定私有路径边界。

#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use rusqlite::Connection;
use studiolo_supply::{
    SupplyError,
    index_cache::{IndexCacheStore, MAX_PAYLOAD_BYTES, validate_isolation},
};
use tempfile::TempDir;

fn private_dir() -> (TempDir, PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("cache");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

fn database(dir: &Path) -> PathBuf {
    dir.join("index.sqlite3")
}
fn scope() -> String {
    "a".repeat(64)
}
fn sql(dir: &Path, statement: &str) {
    Connection::open(database(dir))
        .unwrap()
        .execute_batch(statement)
        .unwrap();
}

#[test]
fn empty_roundtrip_reopen_and_scope_replacement() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    assert_eq!(store.load(&scope()).unwrap(), None);
    store.replace(&scope(), b"fictional-index-v1").unwrap();
    assert_eq!(
        store.load(&scope()).unwrap().unwrap(),
        b"fictional-index-v1"
    );
    drop(store);
    let store = IndexCacheStore::open(&dir).unwrap();
    assert_eq!(
        store.load(&scope()).unwrap().unwrap(),
        b"fictional-index-v1"
    );
    let next = "b".repeat(64);
    assert_eq!(store.load(&next).unwrap(), None);
    store.replace(&next, b"fictional-index-v2").unwrap();
    assert_eq!(store.load(&scope()).unwrap(), None);
    assert_eq!(store.load(&next).unwrap().unwrap(), b"fictional-index-v2");
    let connection = Connection::open(database(&dir)).unwrap();
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM cache_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(
        fs::metadata(database(&dir)).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert!(!dir.join("agent.sqlite3").exists());
}

#[test]
fn replacement_does_not_retain_old_payload_bytes() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    let old = b"fictional-old-source-must-not-remain".repeat(512);
    store.replace(&scope(), &old).unwrap();
    store.replace(&"b".repeat(64), b"new").unwrap();
    let bytes = fs::read(database(&dir)).unwrap();
    assert!(!bytes.windows(34).any(|window| window == &old[..34]));
    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(!dir.join(format!("index.sqlite3{suffix}")).exists());
    }
}

#[test]
fn corruption_is_rejected_without_repair() {
    for mutation in [
        "UPDATE cache_entry SET sha256=zeroblob(32)",
        "UPDATE cache_entry SET bytes=bytes+1",
        "UPDATE cache_entry SET payload='text'",
    ] {
        let (_temp, dir) = private_dir();
        let store = IndexCacheStore::open(&dir).unwrap();
        store.replace(&scope(), b"before").unwrap();
        sql(&dir, mutation);
        let before = fs::read(database(&dir)).unwrap();
        assert!(store.load(&scope()).is_err());
        assert!(store.replace(&scope(), b"after").is_err());
        assert_eq!(fs::read(database(&dir)).unwrap(), before);
    }
}

#[test]
fn malformed_sqlite_unknown_version_and_extra_schema_are_preserved() {
    for mutation in [
        "PRAGMA user_version=99",
        "PRAGMA application_id=7",
        "CREATE TABLE unrelated (value TEXT)",
        "CREATE TRIGGER reject_update BEFORE UPDATE ON cache_entry BEGIN SELECT RAISE(ABORT, 'denied'); END",
    ] {
        let (_temp, dir) = private_dir();
        drop(IndexCacheStore::open(&dir).unwrap());
        sql(&dir, mutation);
        let before = fs::read(database(&dir)).unwrap();
        assert!(matches!(
            IndexCacheStore::open(&dir),
            Err(SupplyError::CorruptStore)
        ));
        assert_eq!(fs::read(database(&dir)).unwrap(), before);
    }
    for data in [b"not-sqlite".as_slice(), b""] {
        let (_temp, dir) = private_dir();
        fs::write(database(&dir), data).unwrap();
        fs::set_permissions(database(&dir), fs::Permissions::from_mode(0o600)).unwrap();
        assert!(IndexCacheStore::open(&dir).is_err());
        assert_eq!(fs::read(database(&dir)).unwrap(), data);
    }
}

#[test]
fn exact_budget_and_invalid_scope_preserve_previous_batch() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    let full = vec![0x5a; MAX_PAYLOAD_BYTES];
    store.replace(&scope(), &full).unwrap();
    assert_eq!(store.load(&scope()).unwrap().unwrap(), full);
    assert!(
        store
            .replace(&scope(), &vec![0; MAX_PAYLOAD_BYTES + 1])
            .is_err()
    );
    for bad in ["", "abc", &"G".repeat(64), &"a".repeat(65)] {
        assert!(store.replace(bad, b"invalid").is_err());
        assert!(store.load(bad).is_err());
    }
    assert_eq!(store.load(&scope()).unwrap().unwrap(), full);
}

#[test]
fn oversized_blob_length_is_rejected() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    store.replace(&scope(), b"small").unwrap();
    sql(
        &dir,
        "PRAGMA ignore_check_constraints=ON; UPDATE cache_entry SET payload=zeroblob(33554433), bytes=33554433",
    );
    assert!(matches!(
        store.load(&scope()),
        Err(SupplyError::CorruptStore)
    ));
}

#[test]
fn refused_transaction_keeps_previous_batch() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    store.replace(&scope(), b"committed").unwrap();
    let connection = Connection::open(database(&dir)).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(store.replace(&scope(), b"uncommitted").is_err());
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(store.load(&scope()).unwrap().unwrap(), b"committed");
}

#[test]
fn changed_schema_after_open_cannot_commit_a_replacement() {
    let (_temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    store.replace(&scope(), b"original").unwrap();
    sql(&dir, "PRAGMA user_version=2");
    assert!(store.replace(&scope(), b"wrong").is_err());
    sql(&dir, "PRAGMA user_version=1");
    assert_eq!(store.load(&scope()).unwrap().unwrap(), b"original");
}

#[test]
fn links_permissions_missing_and_relative_roots_are_rejected() {
    let (temp, dir) = private_dir();
    let absent = temp.path().join("absent");
    assert!(IndexCacheStore::open(&absent).is_err());
    assert!(!absent.exists());
    assert!(IndexCacheStore::open(Path::new("relative-cache")).is_err());
    let link = temp.path().join("link");
    symlink(&dir, &link).unwrap();
    assert!(matches!(
        IndexCacheStore::open(&link),
        Err(SupplyError::UnsafeStorage)
    ));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        IndexCacheStore::open(&dir),
        Err(SupplyError::UnsafeStorage)
    ));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let external = temp.path().join("external");
    fs::write(&external, b"fictional-external").unwrap();
    symlink(&external, database(&dir)).unwrap();
    assert!(matches!(
        IndexCacheStore::open(&dir),
        Err(SupplyError::UnsafeStorage)
    ));
    assert_eq!(fs::read(external).unwrap(), b"fictional-external");
}

#[test]
fn database_hardlinks_permissions_and_sidecars_are_rechecked() {
    let (temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    let linked = temp.path().join("hardlink");
    fs::hard_link(database(&dir), &linked).unwrap();
    assert!(matches!(
        store.load(&scope()),
        Err(SupplyError::UnsafeStorage)
    ));
    fs::remove_file(linked).unwrap();
    fs::set_permissions(database(&dir), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        store.replace(&scope(), b"no"),
        Err(SupplyError::UnsafeStorage)
    ));
    fs::set_permissions(database(&dir), fs::Permissions::from_mode(0o600)).unwrap();
    let sidecar = dir.join("index.sqlite3-journal");
    symlink(database(&dir), &sidecar).unwrap();
    assert!(matches!(
        store.load(&scope()),
        Err(SupplyError::UnsafeStorage)
    ));
    assert!(matches!(
        IndexCacheStore::open(&dir),
        Err(SupplyError::UnsafeStorage)
    ));
}

#[test]
fn database_and_root_rebinding_are_rejected() {
    let (temp, dir) = private_dir();
    let store = IndexCacheStore::open(&dir).unwrap();
    fs::rename(database(&dir), dir.join("moved.sqlite3")).unwrap();
    fs::write(database(&dir), b"replacement").unwrap();
    fs::set_permissions(database(&dir), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        store.load(&scope()),
        Err(SupplyError::UnsafeStorage)
    ));
    let root_moved = temp.path().join("old-root");
    fs::rename(&dir, root_moved).unwrap();
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        store.replace(&scope(), b"no"),
        Err(SupplyError::UnsafeStorage)
    ));
}

#[test]
fn isolation_rejects_both_nesting_directions_and_links_without_writes() {
    let (temp, dir) = private_dir();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    assert!(validate_isolation(&dir, &[&source]).is_ok());
    assert!(validate_isolation(&dir, &[&dir]).is_err());
    assert!(validate_isolation(&dir, &[temp.path()]).is_err());
    let nested = dir.join("source");
    fs::create_dir(&nested).unwrap();
    assert!(validate_isolation(&dir, &[&nested]).is_err());
    let alias = temp.path().join("source-link");
    symlink(source, &alias).unwrap();
    assert!(validate_isolation(&dir, &[&alias]).is_err());
    assert!(!database(&dir).exists());
}

#[test]
fn isolation_rejects_case_alias_when_filesystem_folds_case() {
    use std::os::unix::fs::MetadataExt;
    let (temp, dir) = private_dir();
    let alias = temp.path().join("CACHE");
    if let Ok(metadata) = fs::metadata(&alias) {
        assert_eq!(metadata.ino(), fs::metadata(&dir).unwrap().ino());
        assert!(validate_isolation(&dir, &[&alias]).is_err());
        assert!(validate_isolation(&alias, &[&dir]).is_err());
    }
    assert!(!database(&dir).exists());
}
