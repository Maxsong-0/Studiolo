//! 流式逐表校验，包含真实 rowid、schema 与 SQLite 类型，不整库读入内存。
use crate::{Error, sqlite};
use rusqlite::{Connection, types::ValueRef};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub(crate) fn read(db: &Connection, deadline: Instant) -> Result<[u8; 32], Error> {
    let mut hash = Sha256::new();
    for pragma in ["PRAGMA user_version", "PRAGMA application_id"] {
        let value: i64 = db
            .query_row(pragma, [], |r| r.get(0))
            .map_err(sqlite::error)?;
        hash.update(value.to_le_bytes());
    }
    let mut schema = db
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .map_err(sqlite::error)?;
    let mut rows = schema.query([]).map_err(sqlite::error)?;
    let mut tables = Vec::new();
    let mut count = 0;
    while let Some(row) = rows.next().map_err(sqlite::error)? {
        sqlite::check_deadline(deadline)?;
        count += 1;
        if count > 1024 {
            return Err(Error::Invalid);
        }
        hash.update([0xfc]);
        for column in 0..4 {
            value(&mut hash, row.get_ref(column).map_err(sqlite::error)?);
        }
        let kind: String = row.get(0).map_err(sqlite::error)?;
        let name: String = row.get(1).map_err(sqlite::error)?;
        if name.len() > 256 {
            return Err(Error::Invalid);
        }
        if kind == "table" {
            tables.push(name);
        }
    }
    if tables.is_empty() || tables.len() > 256 {
        return Err(Error::Invalid);
    }
    for name in tables {
        sqlite::check_deadline(deadline)?;
        let (kind, without_rowid): (String, bool) = db
            .query_row(
                "SELECT type,wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [&name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sqlite::error)?;
        if kind != "table" {
            return Err(Error::Invalid);
        }
        let mut columns = db
            .prepare("SELECT name,pk FROM pragma_table_xinfo(?1) ORDER BY cid")
            .map_err(sqlite::error)?;
        let items = columns
            .query_map([&name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(sqlite::error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite::error)?;
        let (select, order) = if without_rowid {
            let mut keys: Vec<_> = items.iter().filter(|(_, index)| *index > 0).collect();
            keys.sort_by_key(|(_, index)| *index);
            if keys.is_empty() {
                return Err(Error::Invalid);
            }
            (
                "*".to_owned(),
                keys.iter()
                    .map(|(name, _)| quote(name))
                    .collect::<Vec<_>>()
                    .join(","),
            )
        } else {
            let rowid = ["rowid", "_rowid_", "oid"]
                .into_iter()
                .find(|candidate| {
                    !items
                        .iter()
                        .any(|(column, _)| column.eq_ignore_ascii_case(candidate))
                })
                .ok_or(Error::Invalid)?;
            (format!("{},*", quote(rowid)), quote(rowid))
        };
        hash.update([0xfd]);
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        let mut table = db
            .prepare(&format!(
                "SELECT {select} FROM {} ORDER BY {order}",
                quote(&name)
            ))
            .map_err(sqlite::error)?;
        let columns = table.column_count();
        hash.update((columns as u64).to_le_bytes());
        let mut rows = table.query([]).map_err(sqlite::error)?;
        while let Some(row) = rows.next().map_err(sqlite::error)? {
            sqlite::check_deadline(deadline)?;
            hash.update([0xfe]);
            for column in 0..columns {
                value(&mut hash, row.get_ref(column).map_err(sqlite::error)?);
            }
        }
    }
    sqlite::check_deadline(deadline)?;
    Ok(hash.finalize().into())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn value(hash: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hash.update([0]),
        ValueRef::Integer(value) => {
            hash.update([1]);
            hash.update(value.to_le_bytes());
        }
        ValueRef::Real(value) => {
            hash.update([2]);
            hash.update(value.to_bits().to_le_bytes());
        }
        ValueRef::Text(value) => {
            hash.update([3]);
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value);
        }
        ValueRef::Blob(value) => {
            hash.update([4]);
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value);
        }
    }
}
