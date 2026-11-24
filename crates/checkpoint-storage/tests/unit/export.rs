//! 导出途中的落盘：够得着没公开的段长和计数，所以挂在 `src/export.rs` 底下编，文件放这里（源码目录不许开库、写文件）。
use super::{Flush, STEP_PAGES, perform};
use crate::{DatabaseFile, Error, FrozenSqlite, Source, paths::FLUSHED};
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

/// `rows` 行、每行 `bytes` 字节的控制库，连同它一共要拷几步。
fn database(dir: &tempfile::TempDir, rows: u32, bytes: u32) -> (Source, u64) {
    let path = dir.path().join("control.sqlite3");
    let db = Connection::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE filler(body BLOB);")
        .unwrap();
    db.execute(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<?1)
         INSERT INTO filler SELECT zeroblob(?2) FROM n WHERE i<=?1",
        (rows, bytes),
    )
    .unwrap();
    let source = Source::bind(dir.path(), DatabaseFile::Control).unwrap();
    let pages: i64 = db
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let steps = u64::try_from(pages)
        .unwrap()
        .div_ceil(STEP_PAGES.unsigned_abs().into());
    (source, steps)
}
fn freeze(source: &Source) -> FrozenSqlite {
    FLUSHED.take();
    source
        .freeze(Instant::now() + Duration::from_secs(60))
        .unwrap()
}

fn flushes() -> u64 {
    FLUSHED.with_borrow(|lengths| lengths.len() as u64)
}

/// 一步拷的字节数：夹具的库用默认的 4 KiB 一页。
const STEP_BYTES: u64 = 64 * 4096;

/// 按给定的段长导出约 2 MiB（500 多页）的库：落盘次数和段长对得上，副本照样核过；段长比库大就一次不落。
#[test]
fn a_large_export_flushes_once_per_segment() {
    let dir = private();
    let (source, steps) = database(&dir, 512, 4000);
    assert!(steps > 4, "夹具太小，数不出段数：{steps} 步");
    for (segment, expected) in [
        // 一步一段：最后一步拷完返回 Done，由收尾的 `sync` 落，不另落。
        (1, steps - 1),
        (2 * STEP_BYTES, (steps - 1) / 2),
        (u64::MAX, 0),
    ] {
        let frozen = freeze(&source);
        let target = private();
        perform(&frozen, target.path(), &512, rows, Flush::every(segment)).unwrap();
        assert_eq!(flushes(), expected, "每 {segment} 字节一段");
    }
}

/// 正式导出用的就是 64 MiB 一段：库刚过一段，中途恰好落一次（BUG-054）；不到一段，一次都不另落（不给小库加活）。
/// 落的那一刻拷过的页真的在主文件里：目标不走 WAL（走的话页在 `-wal` 里，落主文件等于没落），
/// 只差 SQLite 自己的页缓存（默认 2 MB）。
#[test]
fn the_real_export_flushes_every_64_mib() {
    const MIB: u32 = 1 << 20;
    const CACHED: u64 = 4 * MIB as u64;
    for (mebibytes, expected) in [(0, 0), (1, 0), (65, 1)] {
        let dir = private();
        let (source, _) = database(&dir, mebibytes, MIB);
        freeze(&source)
            .export(private().path(), &i64::from(mebibytes), rows)
            .unwrap();
        assert_eq!(flushes(), expected, "{mebibytes} MiB");
    }
    let written = FLUSHED.take();
    assert!(written[0] + CACHED >= 64 * u64::from(MIB), "{written:?}");
}
