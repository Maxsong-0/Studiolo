//! 原四表及自动索引的逐字格式；升级不得重建、修改旧行或重发旧回执。

use super::{SessionError, database::unavailable, schema::APP_ID};
use rusqlite::{Connection, params};

const META: &str = "CREATE TABLE auth_meta (
 id INTEGER PRIMARY KEY CHECK(id=1), config TEXT NOT NULL, store_id TEXT NOT NULL,
 epoch INTEGER NOT NULL CHECK(epoch>0), clock_floor INTEGER NOT NULL CHECK(clock_floor>=0),
 next_sequence INTEGER NOT NULL CHECK(next_sequence>0)
) STRICT";
const SESSIONS: &str = "CREATE TABLE sessions (
 id TEXT PRIMARY KEY, token_digest BLOB NOT NULL UNIQUE, csrf_digest BLOB NOT NULL UNIQUE,
 epoch INTEGER NOT NULL CHECK(epoch>0), state TEXT NOT NULL,
 created INTEGER NOT NULL, last_seen INTEGER NOT NULL, absolute_expiry INTEGER NOT NULL,
 access_expiry INTEGER NOT NULL, grant_id TEXT NOT NULL UNIQUE, grant_receipt TEXT, activated_at INTEGER
) STRICT";
const OUTBOX: &str = "CREATE TABLE audit_outbox (
 sequence INTEGER PRIMARY KEY, event_id TEXT NOT NULL, phase TEXT NOT NULL,
 event_json TEXT NOT NULL, event_digest TEXT NOT NULL, kind TEXT NOT NULL, bucket INTEGER UNIQUE, through_count INTEGER,
 receipt_json TEXT, UNIQUE(event_id,phase)
) STRICT";
const DENIALS: &str = "CREATE TABLE auth_denials (
 kind INTEGER PRIMARY KEY CHECK(kind BETWEEN 0 AND 7), total INTEGER NOT NULL CHECK(total>=0),
 delivered INTEGER NOT NULL CHECK(delivered>=0 AND delivered<=total), last_receipt TEXT
) STRICT";

pub(super) const OBJECTS: &[(&str, Option<&str>)] = &[
    ("auth_meta", Some(META)),
    ("sessions", Some(SESSIONS)),
    ("audit_outbox", Some(OUTBOX)),
    ("auth_denials", Some(DENIALS)),
    ("sqlite_autoindex_sessions_1", None),
    ("sqlite_autoindex_sessions_2", None),
    ("sqlite_autoindex_sessions_3", None),
    ("sqlite_autoindex_sessions_4", None),
    ("sqlite_autoindex_audit_outbox_1", None),
    ("sqlite_autoindex_audit_outbox_2", None),
];

pub(super) fn create(db: &Connection, config: &str, store_id: &str) -> Result<(), SessionError> {
    db.execute_batch(&format!(
        "{META};{SESSIONS};{OUTBOX};{DENIALS};PRAGMA user_version=1;PRAGMA application_id={APP_ID};"
    ))
    .map_err(unavailable)?;
    db.execute(
        "INSERT INTO auth_meta VALUES(1,?1,?2,1,0,1)",
        params![config, store_id],
    )
    .map_err(unavailable)?;
    for kind in 0..8 {
        db.execute("INSERT INTO auth_denials VALUES(?1,0,0,NULL)", [kind])
            .map_err(unavailable)?;
    }
    Ok(())
}
