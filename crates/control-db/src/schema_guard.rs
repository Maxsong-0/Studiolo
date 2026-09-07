//! 迁移准入只读检查；不创建账本、不配置持久 PRAGMA。

use rusqlite::{Connection, OptionalExtension as _};

use crate::{DbError, SCHEMA_VERSION};

pub(super) fn require_supported_version(connection: &Connection) -> Result<(), DbError> {
    let kind: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_schema
             WHERE type IN ('table', 'view')
             AND name = 'schema_migrations' COLLATE NOCASE",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match kind.as_deref() {
        None => return Ok(()),
        Some("table") => {}
        Some(_) => {
            return Err(DbError::Corrupt(
                "schema_migrations is not a table".to_owned(),
            ));
        }
    }
    let recorded_version: i64 = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if recorded_version > SCHEMA_VERSION {
        return Err(DbError::UnsupportedSchema {
            found: recorded_version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/schema_guard/private.rs"]
mod tests;
