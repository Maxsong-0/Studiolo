//! v2 仅附加凭证状态、固定旧摘要集合和有界维护状态，不修改原四表。

use super::{SessionError, credential_store, database::unavailable, retention_schema};
use rusqlite::Connection;

const STATE: &str = "CREATE TABLE credential_state (
 id INTEGER PRIMARY KEY CHECK(id=1), next_issuance INTEGER NOT NULL CHECK(next_issuance>0),
 legacy_count INTEGER NOT NULL CHECK(legacy_count BETWEEN 0 AND 1024), legacy_hash BLOB NOT NULL
) STRICT";
const SEEDS: &str = "CREATE TABLE session_seeds (
 id TEXT PRIMARY KEY, issuance INTEGER UNIQUE, token_seed BLOB NOT NULL, csrf_seed BLOB NOT NULL
) STRICT";
const LEGACY: &str = "CREATE TABLE auth_legacy (
 id TEXT PRIMARY KEY, token_digest BLOB NOT NULL, csrf_digest BLOB NOT NULL
) STRICT";

pub(super) const OBJECTS: &[(&str, Option<&str>)] = &[
    ("credential_state", Some(STATE)),
    ("session_seeds", Some(SEEDS)),
    ("auth_legacy", Some(LEGACY)),
    ("sqlite_autoindex_session_seeds_1", None),
    ("sqlite_autoindex_session_seeds_2", None),
    ("sqlite_autoindex_auth_legacy_1", None),
];

pub(super) fn initialize(db: &Connection) -> Result<(), SessionError> {
    db.execute_batch(&format!("{STATE};{SEEDS};{LEGACY};"))
        .map_err(unavailable)?;
    credential_store::initialize(db)?;
    retention_schema::initialize(db)?;
    db.execute_batch("PRAGMA user_version=2")
        .map_err(unavailable)?;
    Ok(())
}
