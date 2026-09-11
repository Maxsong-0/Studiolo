//! 固定缓存格式；拒绝误选的 SQLite 文件、未知版本与额外 schema 对象。

use rusqlite::Connection;

use crate::SupplyError;

pub(super) const APPLICATION_ID: i64 = 0x5354_4349;
pub(super) const SCHEMA_VERSION: i64 = 1;
pub(super) const TABLE_SQL: &str = "CREATE TABLE cache_entry (
    slot INTEGER PRIMARY KEY CHECK (slot = 1),
    scope TEXT NOT NULL CHECK (length(scope) = 64),
    payload BLOB NOT NULL,
    bytes INTEGER NOT NULL CHECK (bytes BETWEEN 0 AND 33554432),
    sha256 BLOB NOT NULL CHECK (length(sha256) = 32)
)";

pub(super) fn validate_schema(connection: &Connection) -> Result<(), SupplyError> {
    let check = || -> rusqlite::Result<bool> {
        let app: i64 = connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let journal: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let count: i64 =
            connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))?;
        if app != APPLICATION_ID || version != SCHEMA_VERSION || journal != "delete" || count != 1 {
            return Ok(false);
        }
        let sql_length: i64 = connection.query_row(
            "SELECT length(sql) FROM sqlite_schema WHERE type='table' AND name='cache_entry'",
            [],
            |row| row.get(0),
        )?;
        if sql_length != TABLE_SQL.len() as i64 {
            return Ok(false);
        }
        let sql: String = connection.query_row(
            "SELECT sql FROM sqlite_schema WHERE name='cache_entry'",
            [],
            |row| row.get(0),
        )?;
        let rows: i64 =
            connection.query_row("SELECT count(*) FROM cache_entry", [], |row| row.get(0))?;
        let invalid_slots: i64 = connection.query_row(
            "SELECT count(*) FROM cache_entry WHERE slot != 1",
            [],
            |row| row.get(0),
        )?;
        Ok(sql == TABLE_SQL && rows <= 1 && invalid_slots == 0)
    };
    match check() {
        Ok(true) => Ok(()),
        _ => Err(SupplyError::CorruptStore),
    }
}
