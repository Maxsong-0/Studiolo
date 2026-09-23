//! 持久签发序号、保留会话的种子摘要与迁移时冻结的旧凭证摘要集合。

use super::{
    SessionError, SessionId,
    database::{required_row, unavailable},
    retention, schema, secrets,
};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

/// 仅在完整 v1 校验之后、升级事务中调用；旧集合从此不增删。
pub(super) fn initialize(db: &Connection) -> Result<(), SessionError> {
    db.execute(
        "INSERT INTO auth_legacy SELECT id,token_digest,csrf_digest FROM sessions",
        [],
    )
    .map_err(unavailable)?;
    let (count, hash) = legacy_seal(db)?;
    db.execute(
        "INSERT INTO credential_state VALUES(1,1,?1,?2)",
        params![count, hash.as_slice()],
    )
    .map_err(unavailable)?;
    Ok(())
}

/// 调用者必须先单独提交该事务，再调用外部审计；失败允许烧号，不得回收序号。
pub(super) fn reserve(db: &Connection) -> Result<i64, SessionError> {
    if db.is_autocommit() {
        return Err(SessionError::StorageUnavailable);
    }
    let next = next_issuance(db)?;
    let following = next.checked_add(1).ok_or(SessionError::Capacity)?;
    db.execute(
        "UPDATE credential_state SET next_issuance=?1 WHERE id=1",
        [following],
    )
    .map_err(unavailable)?;
    Ok(next)
}

pub(super) fn next_issuance(db: &Connection) -> Result<i64, SessionError> {
    db.query_row(
        "SELECT next_issuance FROM credential_state WHERE id=1",
        [],
        |r| r.get(0),
    )
    .map_err(required_row)
}

/// 与 pending 行在同一事务写入；维护高水位之前的迟到签发不得重新进入数据库。
pub(super) fn insert(
    db: &Connection,
    id: &SessionId,
    issuance: i64,
    token_seed_digest: &[u8; 32],
    csrf_seed_digest: &[u8; 32],
) -> Result<(), SessionError> {
    if db.is_autocommit() || issuance <= 0 || id.as_str() != format!("v2-{issuance:016x}") {
        return Err(SessionError::CorruptStore);
    }
    if issuance >= next_issuance(db)? || issuance < retention::issuance_floor(db)? {
        return Err(SessionError::Rejected);
    }
    db.execute(
        "INSERT INTO session_seeds VALUES(?1,?2,?3,?4)",
        params![
            id.as_str(),
            issuance,
            token_seed_digest.as_slice(),
            csrf_seed_digest.as_slice()
        ],
    )
    .map_err(unavailable)?;
    Ok(())
}

/// 四参依次为 token 种子的 token/csrf 域、csrf 种子的 token/csrf 域摘要。
pub(super) fn collided(
    db: &Connection,
    token_as_token: &[u8; 32],
    token_as_csrf: &[u8; 32],
    csrf_as_token: &[u8; 32],
    csrf_as_csrf: &[u8; 32],
) -> Result<bool, SessionError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_seeds WHERE token_seed IN(?1,?3) OR csrf_seed IN(?2,?4) UNION ALL SELECT 1 FROM auth_legacy WHERE token_digest IN(?1,?3) OR csrf_digest IN(?2,?4))",
        params![token_as_token.as_slice(), token_as_csrf.as_slice(), csrf_as_token.as_slice(), csrf_as_csrf.as_slice()],
        |r| r.get(0),
    ).map_err(unavailable)
}

pub(super) fn validate(db: &Connection) -> Result<(), SessionError> {
    for query in [
        "SELECT count(*) FROM credential_state WHERE id!=1 OR next_issuance<=0 OR legacy_count NOT BETWEEN 0 AND 1024 OR octet_length(legacy_hash)!=32",
        "SELECT count(*) FROM session_seeds WHERE octet_length(id)!=19 OR issuance IS NULL OR issuance<=0 OR octet_length(token_seed)!=32 OR octet_length(csrf_seed)!=32",
    ] {
        let invalid: i64 = db.query_row(query, [], |r| r.get(0)).map_err(unavailable)?;
        if invalid != 0 {
            return Err(SessionError::CorruptStore);
        }
    }
    let count: i64 = db
        .query_row("SELECT count(*) FROM credential_state", [], |r| r.get(0))
        .map_err(unavailable)?;
    let seeds: i64 = db
        .query_row("SELECT count(*) FROM session_seeds", [], |r| r.get(0))
        .map_err(unavailable)?;
    if count != 1 || seeds > schema::MAX_SESSIONS {
        return Err(SessionError::CorruptStore);
    }
    let (next, count, hash): (i64, i64, Vec<u8>) = db
        .query_row(
            "SELECT next_issuance,legacy_count,legacy_hash FROM credential_state WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(required_row)?;
    let (actual_count, actual_hash) = legacy_seal(db)?;
    if count != actual_count
        || !secrets::equal(&hash, &actual_hash)
        || next < retention::issuance_floor(db)?
    {
        return Err(SessionError::CorruptStore);
    }
    let mut statement = db
        .prepare(
            "SELECT z.id,z.issuance,s.id FROM session_seeds z LEFT JOIN sessions s ON s.id=z.id",
        )
        .map_err(unavailable)?;
    let mut values = statement.query([]).map_err(unavailable)?;
    while let Some(row) = values.next().map_err(unavailable)? {
        let id: String = row.get(0).map_err(unavailable)?;
        let issuance: i64 = row.get(1).map_err(unavailable)?;
        let session: Option<String> = row.get(2).map_err(unavailable)?;
        if issuance >= next || id != format!("v2-{issuance:016x}") || session.is_none() {
            return Err(SessionError::CorruptStore);
        }
    }
    associations(db)
}

fn associations(db: &Connection) -> Result<(), SessionError> {
    let mut statement = db.prepare("SELECT s.id,s.token_digest,s.csrf_digest,l.token_digest,l.csrf_digest,z.issuance FROM sessions s LEFT JOIN auth_legacy l ON l.id=s.id LEFT JOIN session_seeds z ON z.id=s.id").map_err(unavailable)?;
    let mut rows = statement.query([]).map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let id: String = row.get(0).map_err(unavailable)?;
        let token: Vec<u8> = row.get(1).map_err(unavailable)?;
        let csrf: Vec<u8> = row.get(2).map_err(unavailable)?;
        let old_token: Option<Vec<u8>> = row.get(3).map_err(unavailable)?;
        let old_csrf: Option<Vec<u8>> = row.get(4).map_err(unavailable)?;
        let issuance: Option<i64> = row.get(5).map_err(unavailable)?;
        let valid = if schema::legacy_id(&id) {
            issuance.is_none()
                && old_token.as_ref() == Some(&token)
                && old_csrf.as_ref() == Some(&csrf)
        } else {
            issuance.is_some() && old_token.is_none() && old_csrf.is_none()
        };
        if !valid {
            return Err(SessionError::CorruptStore);
        }
    }
    Ok(())
}

/// 固定长、规范排序的封印仅检测不一致损坏，不抵御管理员重写整个数据库。
fn legacy_seal(db: &Connection) -> Result<(i64, [u8; 32]), SessionError> {
    let count: i64 = db
        .query_row("SELECT count(*) FROM auth_legacy", [], |r| r.get(0))
        .map_err(unavailable)?;
    if !(0..=schema::MAX_SESSIONS).contains(&count) {
        return Err(SessionError::CorruptStore);
    }
    let invalid: i64 = db.query_row("SELECT count(*) FROM auth_legacy WHERE octet_length(id)!=32 OR id GLOB '*[^0-9a-f]*' OR octet_length(token_digest)!=32 OR octet_length(csrf_digest)!=32", [], |r| r.get(0)).map_err(unavailable)?;
    if invalid != 0 {
        return Err(SessionError::CorruptStore);
    }
    let mut hash = Sha256::new();
    hash.update(b"studiolo-auth-legacy-v2\0");
    hash.update(count.to_be_bytes());
    let mut statement = db
        .prepare("SELECT id,token_digest,csrf_digest FROM auth_legacy ORDER BY id")
        .map_err(unavailable)?;
    let mut rows = statement.query([]).map_err(unavailable)?;
    while let Some(row) = rows.next().map_err(unavailable)? {
        let id: String = row.get(0).map_err(unavailable)?;
        let token: Vec<u8> = row.get(1).map_err(unavailable)?;
        let csrf: Vec<u8> = row.get(2).map_err(unavailable)?;
        hash.update(id.as_bytes());
        hash.update(token);
        hash.update(csrf);
    }
    Ok((count, hash.finalize().into()))
}
