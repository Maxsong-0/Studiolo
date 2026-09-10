//! 先检查 SQLite 长度和类型，再分配完整负载并验证原字节摘要。

use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};

use super::{MAX_PAYLOAD_BYTES, database_error};
use crate::SupplyError;

pub(super) fn validate_scope(scope: &str) -> Result<(), SupplyError> {
    if scope.len() != 64
        || !scope
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SupplyError::CorruptStore);
    }
    Ok(())
}

pub(super) fn read_payload(
    connection: &Connection,
) -> Result<Option<(String, Vec<u8>)>, SupplyError> {
    let row = connection
        .query_row(
            "SELECT length(scope), typeof(scope), length(payload), typeof(payload),
                        bytes, length(sha256), typeof(sha256) FROM cache_entry WHERE slot=1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some((scope_len, scope_type, length, payload_type, bytes, hash_len, hash_type)) = row
    else {
        return Ok(None);
    };
    if scope_len != 64
        || scope_type != "text"
        || payload_type != "blob"
        || !(0..=MAX_PAYLOAD_BYTES as i64).contains(&length)
        || bytes != length
        || hash_len != 32
        || hash_type != "blob"
    {
        return Err(SupplyError::CorruptStore);
    }
    let stored_scope: String = connection
        .query_row("SELECT scope FROM cache_entry WHERE slot=1", [], |row| {
            row.get(0)
        })
        .map_err(database_error)?;
    validate_scope(&stored_scope)?;
    let (payload, checksum): (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT payload, sha256 FROM cache_entry WHERE slot=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(database_error)?;
    if payload.len() as i64 != length || Sha256::digest(&payload).as_slice() != checksum {
        return Err(SupplyError::CorruptStore);
    }
    Ok(Some((stored_scope, payload)))
}
