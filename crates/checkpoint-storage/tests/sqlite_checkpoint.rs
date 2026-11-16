//! 实际 SQLite、第二写者、旧读者和目录身份交错；不用只回 true 的导出桩。
#![cfg(unix)]
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use studiolo_checkpoint_storage::{DatabaseFile, Error, Source};

fn private() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn database() -> (tempfile::TempDir, Connection, Source) {
    let dir = private();
    let path = dir.path().join("control.sqlite3");
    let db = Connection::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE values_table(name TEXT UNIQUE, body BLOB); CREATE TABLE keyed(name TEXT PRIMARY KEY, value INTEGER) WITHOUT ROWID; INSERT INTO values_table(rowid,name,body) VALUES(3,'first',x'0001'),(47,'last',x'01ff'); INSERT INTO keyed VALUES('second',2),('first',1);").unwrap();
    let source = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    (dir, db, source)
}
fn water(db: &Connection) -> Result<i64, Error> {
    db.query_row("SELECT count(*) FROM values_table", [], |r| r.get(0))
        .map_err(|_| Error::Invalid)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

#[test]
fn source_comparison_checks_both_original_directory_and_database_objects() {
    let (dir, db, source) = database();
    let second = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    assert!(source.same_origin(&second).unwrap());
    let (_other, _other_db, other_source) = database();
    assert!(!source.same_origin(&other_source).unwrap());
    drop(db);
    let path = dir.path().join("control.sqlite3");
    fs::rename(&path, dir.path().join("old.sqlite3")).unwrap();
    fs::copy(dir.path().join("old.sqlite3"), &path).unwrap();
    let replacement = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    assert_eq!(source.same_origin(&replacement), Err(Error::UnsafeStorage));
    assert_eq!(replacement.same_origin(&source), Err(Error::UnsafeStorage));
    assert_eq!(source.same_origin(&second), Err(Error::UnsafeStorage));
}

#[test]
fn export_preserves_rowid_gaps_and_blocks_second_writer_until_drop() {
    let (dir, db, source) = database();
    let target = private();
    let frozen = source.freeze(deadline()).unwrap();
    let second = Connection::open(dir.path().join("control.sqlite3")).unwrap();
    second.busy_timeout(Duration::ZERO).unwrap();
    assert!(
        second
            .execute("INSERT INTO values_table(name) VALUES('blocked')", [])
            .is_err()
    );
    assert_eq!(frozen.inspect(water).unwrap(), 2);
    assert!(
        frozen
            .inspect(|db| db
                .execute("DELETE FROM values_table", [])
                .map_err(|_| Error::Invalid))
            .is_err()
    );
    let proof = frozen.export(target.path(), &2, water).unwrap();
    assert!(proof.is_complete());
    let copy = Connection::open(target.path().join("control.sqlite3")).unwrap();
    let ids: String = copy
        .query_row(
            "SELECT group_concat(rowid, ',') FROM values_table ORDER BY rowid",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ids, "3,47");
    assert_eq!(
        copy.query_row("SELECT sum(value) FROM keyed", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 1);
    drop(frozen);
    second
        .execute("INSERT INTO values_table(name) VALUES('after')", [])
        .unwrap();
    assert_eq!(water(&db).unwrap(), 3);
    assert_eq!(water(&copy).unwrap(), 2);
}

#[test]
fn old_reader_prevents_complete_wal_proof_and_no_target_is_published() {
    let (dir, db, source) = database();
    let target = private();
    let reader = Connection::open(dir.path().join("control.sqlite3")).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    assert_eq!(water(&reader).unwrap(), 2);
    db.execute("INSERT INTO values_table(name) VALUES('new')", [])
        .unwrap();
    let frozen = source
        .freeze(Instant::now() + Duration::from_millis(100))
        .unwrap();
    assert_eq!(
        frozen.export(target.path(), &3, water),
        Err(Error::Deadline)
    );
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    drop(frozen);
    reader.execute_batch("ROLLBACK").unwrap();
    let frozen = source.freeze(deadline()).unwrap();
    assert!(
        frozen
            .export(target.path(), &3, water)
            .unwrap()
            .is_complete()
    );
}

#[test]
fn busy_and_expired_freezes_do_not_leave_reserved_locks() {
    let (_dir, db, source) = database();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        source.freeze(Instant::now() + Duration::from_millis(30)),
        Err(Error::Deadline)
    ));
    db.execute_batch("ROLLBACK").unwrap();
    assert!(matches!(
        source.freeze(Instant::now()),
        Err(Error::Deadline)
    ));
    drop(source.freeze(deadline()).unwrap());
    db.execute("INSERT INTO values_table(name) VALUES('free')", [])
        .unwrap();
}

#[test]
fn existing_target_wrong_owner_value_and_source_replacement_are_rejected() {
    let (dir, _db, source) = database();
    let target = private();
    let frozen = source.freeze(deadline()).unwrap();
    assert_eq!(frozen.export(target.path(), &9, water), Err(Error::Invalid));
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    frozen.export(target.path(), &2, water).unwrap();
    assert_eq!(
        frozen.export(target.path(), &2, water),
        Err(Error::UnsafeStorage)
    );
    let path = dir.path().join("control.sqlite3");
    fs::rename(&path, dir.path().join("original")).unwrap();
    fs::File::create(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(source.check(), Err(Error::UnsafeStorage));
    assert!(frozen.inspect(water).is_err());
}

#[test]
fn target_validation_cannot_substitute_different_content() {
    let (_dir, _db, source) = database();
    let target = private();
    let frozen = source.freeze(deadline()).unwrap();
    let visits = std::sync::atomic::AtomicUsize::new(0);
    let result = frozen.export(target.path(), &2, |db| {
        let index = visits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if index == 1 {
            return Ok(3);
        }
        water(db)
    });
    assert_eq!(result, Err(Error::Invalid));
    // 失败残片不能被下一次封存覆盖。
    assert_eq!(
        frozen.export(target.path(), &2, water),
        Err(Error::UnsafeStorage)
    );
}
