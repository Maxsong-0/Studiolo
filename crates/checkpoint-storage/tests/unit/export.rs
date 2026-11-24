//! 导出途中的落盘：够得着没公开的段长和计数，所以挂在 `src/export.rs` 底下编，文件放这里（源码目录不许开库、写文件）。
use super::{Flush, STEP_PAGES, perform};
use crate::{DatabaseFile, Error, Source, paths::FLUSHES};
use rusqlite::Connection;
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    time::{Duration, Instant},
};

fn private() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn rows(db: &Connection) -> Result<i64, Error> {
    db.query_row("SELECT count(*) FROM filler", [], |row| row.get(0))
        .map_err(|_| Error::Invalid)
}

/// 按给定的段长导出约 2 MiB（500 多页）的库：落盘次数和段长对得上，副本照样核过；段长比库大就一次不落。
#[test]
fn a_large_export_flushes_once_per_segment() {
    let dir = private();
    let path = dir.path().join("control.sqlite3");
    let db = Connection::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode=WAL; CREATE TABLE filler(body BLOB);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<512)
         INSERT INTO filler SELECT randomblob(4000) FROM n;",
    )
    .unwrap();
    let source = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    let pages: i64 = db
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let steps = u64::try_from(pages)
        .unwrap()
        .div_ceil(STEP_PAGES.unsigned_abs().into());
    assert!(steps > 4, "夹具太小，数不出段数：{pages} 页");
    drop(db);
    for (segment, expected) in [
        // 一步一段：最后一步拷完返回 Done，由收尾的 `sync` 落，不另落。
        (1, steps - 1),
        (2 * u64::from(STEP_PAGES.unsigned_abs()), (steps - 1) / 2),
        (u64::MAX, 0),
    ] {
        let frozen = source
            .freeze(Instant::now() + Duration::from_secs(30))
            .unwrap();
        let target = private();
        FLUSHES.set(0);
        perform(&frozen, target.path(), &512, rows, Flush::every(segment)).unwrap();
        assert_eq!(FLUSHES.get(), expected, "每 {segment} 页一段");
    }
}

/// 正式导出用默认段长：2 MiB 的库不到一段，一次都不另落（不给小库加活）。
#[test]
fn a_small_export_is_not_flushed_midway() {
    let dir = private();
    let path = dir.path().join("control.sqlite3");
    let db = Connection::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE filler(body BLOB);")
        .unwrap();
    let source = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    drop(db);
    let frozen = source
        .freeze(Instant::now() + Duration::from_secs(30))
        .unwrap();
    FLUSHES.set(0);
    frozen.export(private().path(), &0, rows).unwrap();
    assert_eq!(FLUSHES.get(), 0);
}
