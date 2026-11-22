//! 唯一连接打开点；冻结连接不借给业务回调，读连接强制 query_only。
use crate::{Error, FrozenSqlite, paths::Binding};
use rusqlite::{Connection, ErrorCode, OpenFlags, config::DbConfig};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use studiolo_core::checkpoint::WalCheckpointProof;

pub(crate) fn connect(
    binding: &Binding,
    read_only: bool,
    deadline: Instant,
) -> Result<Connection, Error> {
    check_deadline(deadline)?;
    binding.check()?;
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let connection = Connection::open_with_flags(
        &binding.path,
        access
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )
    .map_err(error)?;
    connection.busy_timeout(Duration::ZERO).map_err(error)?;
    connection
        .set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .map_err(error)?;
    connection
        .execute_batch(
            "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
        )
        .map_err(error)?;
    limit(&connection, deadline)?;
    binding.check()?;
    check_deadline(deadline)?;
    Ok(connection)
}
/// 连接上的每条语句到截止时间就中止；放开写锁之后换成放开后的截止时间也走这里。
pub(crate) fn limit(connection: &Connection, deadline: Instant) -> Result<(), Error> {
    connection
        .progress_handler(1000, Some(move || Instant::now() >= deadline))
        .map_err(error)
}
pub(crate) fn freeze(binding: Arc<Binding>, deadline: Instant) -> Result<FrozenSqlite, Error> {
    let reserve = connect(&binding, false, deadline)?;
    let mode: String = reserve
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(error)?;
    if mode != "wal" {
        return Err(Error::Invalid);
    }
    loop {
        check_deadline(deadline)?;
        match reserve.execute_batch("BEGIN IMMEDIATE") {
            Ok(()) => break,
            Err(e) if busy(&e) => pause(deadline)?,
            Err(e) => return Err(deadline_error(error(e), deadline)),
        }
    }
    // 写锁在手、没有别的写者：先把 WAL 整个回写主库，再定格。checkpoint 不能在开着事务的连接上做，
    // 所以另用不外借的短命连接，定格放在它之后。
    let wal = checkpoint(&binding, deadline)?;
    // owner 回调与 backup 源都只读。开读事务并真读一页，SQLite 才在这一刻取快照；之后别人写的、
    // 回写的都看不见，backup 也用这个事务读（BUG-045）。
    let source = connect(&binding, true, deadline)?;
    source
        .execute_batch("PRAGMA query_only=ON; BEGIN")
        .map_err(error)?;
    source
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|e| deadline_error(error(e), deadline))?;
    let frozen = FrozenSqlite {
        reserve: Some(reserve),
        source,
        binding,
        deadline,
        wal,
    };
    frozen.check()?;
    Ok(frozen)
}
/// 写锁在手时做 PASSIVE checkpoint，直到 WAL 全部回写；还有旧读者挡着就等到截止时间。
fn checkpoint(binding: &Binding, deadline: Instant) -> Result<WalCheckpointProof, Error> {
    let checkpoint = connect(binding, false, deadline)?;
    loop {
        binding.check()?;
        let proof = checkpoint.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok(WalCheckpointProof {
                busy: row.get(0)?,
                log_frames: row.get(1)?,
                checkpointed_frames: row.get(2)?,
            })
        });
        match proof {
            Ok(proof) if proof.is_complete() => return Ok(proof),
            Ok(_) => pause(deadline)?,
            Err(e) if busy(&e) => pause(deadline)?,
            Err(e) => return Err(deadline_error(error(e), deadline)),
        }
    }
}
pub(crate) fn check_deadline(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::Deadline)
    } else {
        Ok(())
    }
}
pub(crate) fn deadline_error(error: Error, deadline: Instant) -> Error {
    if Instant::now() >= deadline {
        Error::Deadline
    } else {
        error
    }
}
pub(crate) fn pause(deadline: Instant) -> Result<(), Error> {
    check_deadline(deadline)?;
    thread::sleep(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(5)),
    );
    Ok(())
}
pub(crate) fn busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}
pub(crate) fn error(_: rusqlite::Error) -> Error {
    Error::Unavailable
}
