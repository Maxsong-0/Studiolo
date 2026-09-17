//! 仅已验证的数据库可进入写入配置；物理预算参数逐项读回。
use super::{BridgeError, database, model::*};
use rusqlite::Connection;

pub(super) fn connection(db: &Connection) -> Result<()> {
    db.execute_batch("PRAGMA synchronous=FULL;PRAGMA fullfsync=ON;PRAGMA checkpoint_fullfsync=ON;PRAGMA cache_spill=OFF;PRAGMA temp_store=MEMORY;").map_err(database::error)
}
pub(super) fn verify(db: &Connection) -> Result<()> {
    for (sql, expected) in [
        ("PRAGMA synchronous", 2),
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA trusted_schema", 0),
        ("PRAGMA fullfsync", 1),
        ("PRAGMA checkpoint_fullfsync", 1),
        ("PRAGMA page_size", 4096),
        ("PRAGMA max_page_count", 65536),
        ("PRAGMA journal_size_limit", 0),
        ("PRAGMA wal_autocheckpoint", 1),
        ("PRAGMA cache_spill", 0),
        ("PRAGMA temp_store", 2),
    ] {
        let actual: i64 = db
            .query_row(sql, [], |r| r.get(0))
            .map_err(database::error)?;
        if actual != expected {
            return Err(BridgeError::Unavailable);
        }
    }
    if !db
        .db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE)
        .map_err(database::error)?
    {
        return Err(BridgeError::Unavailable);
    }
    Ok(())
}
