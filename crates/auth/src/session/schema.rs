//! 固定版本结构与有界完整校验；原四表格式保持不变，迁移只添加私有表。

use super::{
    SessionError,
    database::{Migration, required_row, unavailable},
    schema_v1, schema_v2,
};
use rusqlite::{Connection, params};

pub(super) const MAX_SESSIONS: i64 = 1024;
pub(super) const MAX_SECURITY_EVENTS: i64 = 8192;
pub(super) const MAX_EVENT_BYTES: i64 = 8192;
pub(super) const APP_ID: i64 = 0x5354_4153;

pub(super) fn initialize(
    db: &Connection,
    created: bool,
    config: &str,
    store_id: Option<&str>,
    migrate: Migration,
) -> Result<(), SessionError> {
    let count: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(unavailable)?;
    if created && count == 0 {
        schema_v1::create(db, config, store_id.ok_or(SessionError::CorruptStore)?)?;
        super::migration::v1(db, config)?;
        if matches!(migrate, Migration::TotpV2 | Migration::RecoveryV3) {
            super::migration::v2(db, config)?;
        }
        if migrate == Migration::RecoveryV3 {
            super::migration::v3(db, config)?;
        }
        return Ok(());
    }
    match version(db)? {
        1 if migrate == Migration::SessionV1 => super::migration::v1(db, config),
        1 => {
            validate(db, config)?;
            Err(SessionError::MigrationRequired)
        }
        2 if migrate == Migration::TotpV2 => super::migration::v2(db, config),
        2 if migrate == Migration::RecoveryV3 => {
            validate(db, config)?;
            Err(SessionError::MigrationRequired)
        }
        3 if migrate == Migration::RecoveryV3 => super::migration::v3(db, config),
        2..=4 => validate(db, config),
        _ => Err(SessionError::UnsupportedSchema),
    }
}

pub(super) fn version(db: &Connection) -> Result<i64, SessionError> {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(unavailable)
}

pub(super) fn validate(db: &Connection, config: &str) -> Result<(), SessionError> {
    let version = version(db)?;
    let application: i64 = db
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(unavailable)?;
    if !matches!(version, 1..=4) || application != APP_ID {
        return Err(SessionError::UnsupportedSchema);
    }
    let mut expected = schema_v1::OBJECTS.to_vec();
    if version >= 2 {
        expected.extend_from_slice(schema_v2::OBJECTS);
        expected.extend_from_slice(super::retention_schema::OBJECTS);
    }
    if version >= 3 {
        expected.extend_from_slice(super::totp::schema::OBJECTS);
    }
    if version == 4 {
        expected.extend_from_slice(super::recovery::schema::OBJECTS);
    }
    structure(db, &mut expected)?;
    let check: String = db
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(unavailable)?;
    if check != "ok" {
        return Err(SessionError::CorruptStore);
    }
    let valid: i64 = db.query_row("SELECT count(*) FROM auth_meta WHERE id=1 AND config=?1 AND epoch>0 AND clock_floor>=0 AND next_sequence>0 AND octet_length(store_id)=32 AND store_id NOT GLOB '*[^0-9a-f]*'", [config], |r| r.get(0)).map_err(unavailable)?;
    if valid != 1 {
        return Err(SessionError::Configuration);
    }
    validate_rows(db, version)?;
    if version >= 2 {
        super::credential_store::validate(db)?;
    }
    super::rows::validate(db, config)?;
    super::outbox::validate(db)?;
    if version >= 2 {
        super::retention_schema::validate(db)?;
    }
    if version >= 3 {
        super::totp::schema::validate(db)?;
    }
    if version == 4 {
        super::recovery::schema::validate(db)?;
    }
    Ok(())
}

fn structure(db: &Connection, expected: &mut [(&str, Option<&str>)]) -> Result<(), SessionError> {
    let count: i64 = db
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(unavailable)?;
    if count != expected.len() as i64 {
        return Err(SessionError::CorruptStore);
    }
    let invalid: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE octet_length(name)>64 OR octet_length(type)>5 OR octet_length(sql)>2048", [], |r| r.get(0)).map_err(unavailable)?;
    if invalid != 0 {
        return Err(SessionError::CorruptStore);
    }
    let mut statement = db
        .prepare("SELECT name,sql FROM sqlite_schema ORDER BY name")
        .map_err(unavailable)?;
    let actual = statement
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)?;
    expected.sort_by(|a, b| a.0.cmp(b.0));
    if actual
        .iter()
        .zip(expected.iter())
        .any(|(actual, expected)| actual.0 != expected.0 || actual.1.as_deref() != expected.1)
    {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn store_id(db: &Connection) -> Result<String, SessionError> {
    db.query_row("SELECT store_id FROM auth_meta WHERE id=1", [], |r| {
        r.get(0)
    })
    .map_err(required_row)
}

fn validate_rows(db: &Connection, version: i64) -> Result<(), SessionError> {
    let sessions: i64 = db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .map_err(unavailable)?;
    let security: i64 = db
        .query_row(
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    let denied: i64 = db
        .query_row(
            "SELECT count(*) FROM audit_outbox WHERE bucket IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    if sessions > MAX_SESSIONS || security > MAX_SECURITY_EVENTS || denied > 8 {
        return Err(SessionError::CorruptStore);
    }
    for query in [
        "SELECT count(*) FROM sessions WHERE octet_length(id)>32 OR octet_length(token_digest)!=32 OR octet_length(csrf_digest)!=32 OR epoch<=0 OR epoch>(SELECT epoch FROM auth_meta) OR state NOT IN ('pending','active','revoked','expired','abandoned') OR created<0 OR last_seen<created OR absolute_expiry<=created OR last_seen>=absolute_expiry OR access_expiry<absolute_expiry OR octet_length(grant_id)>80 OR (state='active' AND grant_receipt IS NULL) OR octet_length(grant_receipt)>8192",
        "SELECT count(*) FROM audit_outbox WHERE sequence<=0 OR sequence>=(SELECT next_sequence FROM auth_meta) OR octet_length(event_id) NOT BETWEEN 1 AND 96 OR phase NOT IN ('intent','outcome') OR octet_length(event_json) NOT BETWEEN 1 AND 8192 OR octet_length(event_digest)!=64 OR octet_length(receipt_json)>8192 OR kind NOT IN ('issue','revoke','expire','denied') OR (bucket IS NOT NULL AND (bucket NOT BETWEEN 0 AND 7 OR kind!='denied' OR through_count<=0)) OR (bucket IS NULL AND kind='denied')",
        "SELECT count(*) FROM auth_denials WHERE kind NOT BETWEEN 0 AND 7 OR total<0 OR delivered<0 OR delivered>total OR octet_length(last_receipt)>8192",
    ] {
        let invalid: i64 = db.query_row(query, [], |r| r.get(0)).map_err(unavailable)?;
        if invalid != 0 {
            return Err(SessionError::CorruptStore);
        }
    }
    let mut statement = db.prepare("SELECT id FROM sessions").map_err(unavailable)?;
    let ids = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(unavailable)?;
    for id in ids {
        let id = id.map_err(unavailable)?;
        if (version == 1 && !legacy_id(&id)) || super::SessionId::parse(id).is_err() {
            return Err(SessionError::CorruptStore);
        }
    }
    let buckets: i64 = db
        .query_row("SELECT count(*) FROM auth_denials", [], |r| r.get(0))
        .map_err(unavailable)?;
    if buckets != 8 {
        return Err(SessionError::CorruptStore);
    }
    Ok(())
}

pub(super) fn legacy_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn next_sequence(db: &Connection) -> Result<i64, SessionError> {
    let next: i64 = db
        .query_row("SELECT next_sequence FROM auth_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(required_row)?;
    let following = next.checked_add(1).ok_or(SessionError::Capacity)?;
    db.execute(
        "UPDATE auth_meta SET next_sequence=?1 WHERE id=1",
        params![following],
    )
    .map_err(unavailable)?;
    Ok(next)
}
