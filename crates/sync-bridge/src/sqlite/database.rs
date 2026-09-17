//! schema v1 固定登记；不接受未知结构/迁移或默认初始化。
use super::{
    BridgeError,
    dbwrite::{full, meta_bytes},
    model::*,
};
pub(super) use super::{dbload::load, dbwrite::persist};
use rusqlite::Connection;
const META: &str = "CREATE TABLE bridge_meta(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL CHECK(revision>=0),encoded_total INTEGER NOT NULL CHECK(encoded_total BETWEEN 0 AND 67108864),payload BLOB NOT NULL CHECK(length(payload)<=67108864))";
const PLANS: &str = "CREATE TABLE bridge_plans(sequence INTEGER PRIMARY KEY CHECK(sequence>0),role TEXT NOT NULL UNIQUE CHECK(role IN ('pending','last','fuse')),input BLOB NOT NULL CHECK(length(input)<=16777216),ack TEXT CHECK(ack IS NULL OR length(CAST(ack AS BLOB))<=8192))";
const ACTIONS: &str = "CREATE TABLE bridge_actions(sequence INTEGER NOT NULL REFERENCES bridge_plans(sequence),position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 4095),payload BLOB NOT NULL CHECK(length(payload)<=32768),PRIMARY KEY(sequence,position))";

pub(super) fn initialize(db: &Connection, state: &State) -> Result<()> {
    for sql in [META, PLANS, ACTIONS] {
        db.execute_batch(sql).map_err(error)?;
    }
    db.execute_batch("PRAGMA user_version=1;PRAGMA application_id=0;")
        .map_err(error)?;
    db.execute(
        "INSERT INTO bridge_meta VALUES(1,0,0,?1)",
        [meta_bytes(state)?],
    )
    .map_err(error)?;
    full(db, state)
}
pub(super) fn schema(db: &Connection) -> Result<()> {
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(error)?;
    if version != 1 {
        return Err(BridgeError::UnsupportedSchema);
    }
    let application: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(error)?;
    let page: i64 = db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(error)?;
    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(error)?;
    if application != 0 || page != 4096 || mode != "wal" {
        return Err(BridgeError::CorruptStore);
    }
    let count: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(error)?;
    if count != 5 {
        return Err(BridgeError::CorruptStore);
    }
    for (name, expected) in [
        ("bridge_meta", META),
        ("bridge_plans", PLANS),
        ("bridge_actions", ACTIONS),
    ] {
        let sql: Option<String> = db.query_row("SELECT CASE WHEN length(CAST(sql AS BLOB))<=1024 THEN sql END FROM sqlite_schema WHERE type='table' AND name=?1 AND tbl_name=?1",[name],|r|r.get(0)).map_err(error)?;
        if sql.as_deref() != Some(expected) {
            return Err(BridgeError::CorruptStore);
        }
    }
    for (index, table) in [
        ("sqlite_autoindex_bridge_plans_1", "bridge_plans"),
        ("sqlite_autoindex_bridge_actions_1", "bridge_actions"),
    ] {
        let valid:i64=db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name=?1 AND tbl_name=?2 AND sql IS NULL AND rootpage>0",rusqlite::params![index,table],|r|r.get(0)).map_err(error)?;
        if valid != 1 {
            return Err(BridgeError::CorruptStore);
        }
    }
    Ok(())
}
pub(super) fn integrity(db: &Connection) -> Result<()> {
    let mut quick = db.prepare("PRAGMA quick_check(1)").map_err(error)?;
    let mut rows = quick.query([]).map_err(error)?;
    let row = rows
        .next()
        .map_err(error)?
        .ok_or(BridgeError::CorruptStore)?;
    if row.get_ref(0).map_err(error)? != rusqlite::types::ValueRef::Text(b"ok")
        || rows.next().map_err(error)?.is_some()
    {
        return Err(BridgeError::CorruptStore);
    }
    let mut foreign = db.prepare("PRAGMA foreign_key_check").map_err(error)?;
    if foreign
        .query([])
        .map_err(error)?
        .next()
        .map_err(error)?
        .is_some()
    {
        return Err(BridgeError::CorruptStore);
    }
    Ok(())
}
pub(super) fn revision(db: &Connection) -> Result<u64> {
    let n: i64 = db
        .query_row("SELECT revision FROM bridge_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(error)?;
    u64::try_from(n).map_err(|_| BridgeError::CorruptStore)
}
pub(super) fn error(_: rusqlite::Error) -> BridgeError {
    BridgeError::Unavailable
}
