//! 封存只从冻结时定格的读连接导出；目标独占创建，失败残片不作为成功票据。
//! 导出可以在放开写锁之后做（BUG-045）：定格的读事务读到的始终是冻结那一刻，别人之后写的、回写的都看不见。
use crate::{Error, FrozenSqlite, digest, paths::Binding, sqlite};
use rusqlite::{
    Connection,
    backup::{Backup, StepResult},
};
use std::path::Path;
use studiolo_core::{Flush, checkpoint::WalCheckpointProof};

/// 备份一步拷的页数。
const STEP_PAGES: i32 = 64;

pub(crate) fn export<T: Eq>(
    frozen: &FrozenSqlite,
    target: &Path,
    expected: &T,
    inspect: impl Fn(&Connection) -> Result<T, Error>,
) -> Result<WalCheckpointProof, Error> {
    frozen.check()?;
    perform(frozen, target, expected, inspect, Flush::default())
        .map_err(|error| sqlite::deadline_error(error, frozen.deadline))
}
fn perform<T: Eq>(
    frozen: &FrozenSqlite,
    target: &Path,
    expected: &T,
    inspect: impl Fn(&Connection) -> Result<T, Error>,
    mut flush: Flush,
) -> Result<WalCheckpointProof, Error> {
    // 定格的源只读一遍全内容摘要；完整性、owner 校验都在副本上做，副本和源逐表摘要相同才算拷对。
    let expected_digest = digest::read(&frozen.source, frozen.deadline)?;
    let step_bytes = page_bytes(&frozen.source)?.saturating_mul(STEP_PAGES.unsigned_abs().into());
    let target = Binding::target(target, &frozen.binding)?;
    let mut destination = sqlite::connect(&target, false, frozen.deadline)?;
    {
        // 源连接开着读事务：backup 用这个事务读，不自己另开、也不因别人写了而从头重来。
        let backup = Backup::new(&frozen.source, &mut destination).map_err(sqlite::error)?;
        loop {
            frozen.check()?;
            target.check()?;
            match backup.step(STEP_PAGES).map_err(sqlite::error)? {
                StepResult::Done => break,
                // 没落盘的量封顶，不随库变大（BUG-054）；拷完的最后一段由下面的 `sync` 落。
                StepResult::More => flush.written(step_bytes, || target.flush())?,
                StepResult::Busy | StepResult::Locked => sqlite::pause(frozen.deadline)?,
                _ => return Err(Error::Unavailable),
            }
        }
    }
    // 独立目标最终只依赖主文件，不把 WAL/SHM 当作传输所需的隐藏对象。
    let mode: String = destination
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .map_err(sqlite::error)?;
    if mode != "delete" {
        return Err(Error::Invalid);
    }
    destination.close().map_err(|_| Error::Unavailable)?;
    target.sync()?;
    if target.has_sidecars()? {
        return Err(Error::Invalid);
    }
    let reopened = sqlite::connect(&target, true, frozen.deadline)?;
    integrity(&reopened)?;
    if inspect(&reopened)? != *expected
        || digest::read(&reopened, frozen.deadline)? != expected_digest
    {
        return Err(Error::Invalid);
    }
    reopened.close().map_err(|_| Error::Unavailable)?;
    // 拷贝和校验可能很久：返回前再核源身份没换、定格的读事务还在。
    frozen.check()?;
    target.sync()?;
    sqlite::check_deadline(frozen.deadline)?;
    Ok(frozen.wal)
}
fn page_bytes(db: &Connection) -> Result<u64, Error> {
    let bytes: i64 = db
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(sqlite::error)?;
    u64::try_from(bytes).map_err(|_| Error::Invalid)
}
fn integrity(db: &Connection) -> Result<(), Error> {
    let result: String = db
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(sqlite::error)?;
    if result != "ok" {
        return Err(Error::Invalid);
    }
    let mut foreign = db
        .prepare("PRAGMA foreign_key_check")
        .map_err(sqlite::error)?;
    if foreign
        .query([])
        .map_err(sqlite::error)?
        .next()
        .map_err(sqlite::error)?
        .is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "../tests/unit/export.rs"]
mod tests;
