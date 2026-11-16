//! 唯一连接打开点；冻结连接不借给业务回调，读连接强制 query_only。
use crate::{Error, FrozenSqlite, paths::Binding};
use rusqlite::{Connection, ErrorCode, OpenFlags, config::DbConfig};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

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
    connection
        .progress_handler(1000, Some(move || Instant::now() >= deadline))
        .map_err(error)?;
    binding.check()?;
    check_deadline(deadline)?;
    Ok(connection)
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
    // owner 回调与 backup 源都只读；WAL checkpoint 另用不外借的短命连接。
    let source = connect(&binding, true, deadline)?;
    source
        .execute_batch("PRAGMA query_only=ON")
        .map_err(error)?;
    let frozen = FrozenSqlite {
        reserve,
        source,
        binding,
        deadline,
    };
    frozen.check()?;
    Ok(frozen)
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
