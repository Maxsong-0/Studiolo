//! 封存只能从完整冻结窗口导出；目标独占创建，失败残片不作为成功票据。
use crate::{Error, FrozenSqlite, digest, paths::Binding, sqlite};
use rusqlite::{
    Connection,
    backup::{Backup, StepResult},
};
use std::path::Path;
use studiolo_core::checkpoint::WalCheckpointProof;

pub(crate) fn export<T: Eq>(
    frozen: &FrozenSqlite,
    target: &Path,
    expected: &T,
    inspect: impl Fn(&Connection) -> Result<T, Error>,
) -> Result<WalCheckpointProof, Error> {
    frozen.check()?;
    if &frozen.inspect(&inspect)? != expected {
        return Err(Error::Invalid);
    }
    let result = perform(frozen, target, expected, inspect);
    result.map_err(|error| sqlite::deadline_error(error, frozen.deadline))
}
fn perform<T: Eq>(
    frozen: &FrozenSqlite,
    target: &Path,
    expected: &T,
    inspect: impl Fn(&Connection) -> Result<T, Error>,
) -> Result<WalCheckpointProof, Error> {
    let checkpoint = sqlite::connect(&frozen.binding, false, frozen.deadline)?;
    let wal = loop {
        frozen.check()?;
        let proof = checkpoint.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok(WalCheckpointProof {
                busy: row.get(0)?,
                log_frames: row.get(1)?,
                checkpointed_frames: row.get(2)?,
            })
        });
        match proof {
            Ok(proof) if proof.is_complete() => break proof,
            Ok(_) => sqlite::pause(frozen.deadline)?,
            Err(error) if sqlite::busy(&error) => sqlite::pause(frozen.deadline)?,
            Err(error) => return Err(sqlite::error(error)),
        }
    };
    drop(checkpoint);
    integrity(&frozen.source)?;
    let expected_digest = digest::read(&frozen.source, frozen.deadline)?;
    let target = Binding::target(target, &frozen.binding)?;
    let mut destination = sqlite::connect(&target, false, frozen.deadline)?;
    {
        let backup = Backup::new(&frozen.source, &mut destination).map_err(sqlite::error)?;
        loop {
            frozen.check()?;
            target.check()?;
            match backup.step(64).map_err(sqlite::error)? {
                StepResult::Done => break,
                StepResult::More => {}
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
    // 业务验证/复制可能很久，返回前再核源，不能把替换对象误当同一库。
    frozen.check()?;
    if frozen.inspect(inspect)? != *expected
        || digest::read(&frozen.source, frozen.deadline)? != expected_digest
    {
        return Err(Error::Invalid);
    }
    target.sync()?;
    sqlite::check_deadline(frozen.deadline)?;
    Ok(wal)
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
