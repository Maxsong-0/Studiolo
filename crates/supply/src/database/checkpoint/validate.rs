//! 不初始化身份库；按行校验格式与父链，摘要不聚合全部实例到内存。
use super::super::{children, schema, validate_type_name};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use studiolo_checkpoint_storage::Error;
use studiolo_core::checkpoint::OwnerWatermark;

pub(super) fn snapshot(db: &Connection) -> Result<OwnerWatermark, Error> {
    schema::validate(db).map_err(invalid)?;
    let version = schema::version(db).map_err(invalid)?;
    let mut digest = Sha256::new();
    digest.update(b"studiolo-agent-types-v1\0");
    let mut statement=db.prepare("SELECT CASE WHEN octet_length(name) BETWEEN 1 AND 32 THEN name END,CASE WHEN octet_length(token_sha256)=64 THEN token_sha256 END,CASE WHEN octet_length(issued_at) BETWEEN 1 AND 64 THEN issued_at END,CASE WHEN revoked_at IS NULL OR octet_length(revoked_at) BETWEEN 1 AND 64 THEN revoked_at ELSE '' END FROM agent_type ORDER BY name").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let name: String = row.get(0).map_err(invalid)?;
        validate_type_name(&name).map_err(invalid)?;
        let token: String = row.get(1).map_err(invalid)?;
        hash(&token)?;
        let issued: String = row.get(2).map_err(invalid)?;
        timestamp(&issued)?;
        let revoked: Option<String> = row.get(3).map_err(invalid)?;
        if let Some(value) = &revoked {
            timestamp(value)?;
        }
        for value in [&name, &token, &issued] {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value.as_bytes());
        }
        match revoked {
            None => digest.update([0]),
            Some(value) => {
                digest.update([1]);
                digest.update((value.len() as u64).to_le_bytes());
                digest.update(value.as_bytes());
            }
        }
    }
    let mut count = 0_u64;
    let mut statement=db.prepare("SELECT CASE WHEN octet_length(id)=26 THEN id END,CASE WHEN octet_length(session_digest)=64 THEN session_digest END,CASE WHEN octet_length(registered_at) BETWEEN 1 AND 64 THEN registered_at END,CASE WHEN octet_length(last_seen_at) BETWEEN 1 AND 64 THEN last_seen_at END FROM agent_instance ORDER BY id").map_err(invalid)?;
    let mut rows = statement.query([]).map_err(invalid)?;
    while let Some(row) = rows.next().map_err(invalid)? {
        let id: String = row.get(0).map_err(invalid)?;
        id.parse::<ulid::Ulid>().map_err(invalid)?;
        hash(&row.get::<_, String>(1).map_err(invalid)?)?;
        timestamp(&row.get::<_, String>(2).map_err(invalid)?)?;
        timestamp(&row.get::<_, String>(3).map_err(invalid)?)?;
        children::lineage(db, &id).map_err(invalid)?;
        count = count.checked_add(1).ok_or(Error::Invalid)?;
    }
    // 开放 outage 表示接入方尚未完成故障后恢复，不能宣称语义已排空。
    let pending: i64 = db
        .query_row(
            "SELECT count(*) FROM outage WHERE ended_at IS NULL OR denied_count<0",
            [],
            |r| r.get(0),
        )
        .map_err(invalid)?;
    if pending != 0 {
        return Err(Error::Invalid);
    }
    let type_token_digest = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(OwnerWatermark::AgentStore {
        schema_version: version as u32,
        type_token_digest,
        instance_count: count,
    })
}
fn hash(value: &str) -> Result<(), Error> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}
fn timestamp(value: &str) -> Result<(), Error> {
    value
        .parse::<jiff::Timestamp>()
        .map(|_| ())
        .map_err(invalid)
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
