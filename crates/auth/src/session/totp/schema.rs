//! v3 仅附加五张固定表；先检查原始 SQL 值，再解析有界决定和回执。

#[path = "validate.rs"]
mod validate;

use crate::session::{
    SessionError,
    database::unavailable,
    verification_rate::{self, Scope},
};
use rusqlite::Connection;

const CONTROL: &str = "CREATE TABLE totp_control (
 id INTEGER PRIMARY KEY CHECK(id=1), next_operation INTEGER NOT NULL CHECK(next_operation>0)
) STRICT";
const FACTOR: &str = "CREATE TABLE totp_factor (
 id INTEGER PRIMARY KEY CHECK(id=1), seed_ref TEXT NOT NULL, seed_digest TEXT NOT NULL,
 last_step INTEGER NOT NULL, consumption_json TEXT NOT NULL, enrollment_json TEXT,
 intent_receipt TEXT, outcome_receipt TEXT, confirmed_at INTEGER
) STRICT";
const PENDING: &str = "CREATE TABLE totp_pending (
 id INTEGER PRIMARY KEY CHECK(id=1), operation_json TEXT NOT NULL,
 intent_receipt TEXT, outcome_receipt TEXT
) STRICT";
const GRANTS: &str = "CREATE TABLE totp_grants (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
 operation_json TEXT NOT NULL, intent_receipt TEXT NOT NULL,
 outcome_receipt TEXT NOT NULL, activated_at INTEGER NOT NULL
) STRICT";
const RATES: &str = "CREATE TABLE totp_rates (
 bucket TEXT PRIMARY KEY, window_started INTEGER NOT NULL, attempts INTEGER NOT NULL
) STRICT";

pub(in crate::session) const OBJECTS: &[(&str, Option<&str>)] = &[
    ("totp_control", Some(CONTROL)),
    ("totp_factor", Some(FACTOR)),
    ("totp_pending", Some(PENDING)),
    ("totp_grants", Some(GRANTS)),
    ("totp_rates", Some(RATES)),
    ("sqlite_autoindex_totp_grants_1", None),
    ("sqlite_autoindex_totp_rates_1", None),
];

pub(in crate::session) fn initialize(db: &Connection) -> Result<(), SessionError> {
    db.execute_batch(&format!("{CONTROL};{FACTOR};{PENDING};{GRANTS};{RATES};INSERT INTO totp_control VALUES(1,1);PRAGMA user_version=3;"))
        .map_err(unavailable)
}

pub(in crate::session) fn validate(db: &Connection) -> Result<(), SessionError> {
    for (table, maximum, required) in [
        ("totp_control", 1_i64, true),
        ("totp_factor", 1, false),
        ("totp_pending", 1, false),
        ("totp_grants", 1024, false),
        ("totp_rates", 66, false),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(unavailable)?;
        if count > maximum || (required && count != 1) {
            return Err(SessionError::CorruptStore);
        }
    }
    for query in [
        "SELECT count(*) FROM totp_control WHERE id!=1 OR typeof(next_operation)!='integer' OR next_operation<=0",
        "SELECT count(*) FROM totp_factor WHERE id!=1 OR typeof(seed_ref)!='text' OR octet_length(seed_ref) NOT BETWEEN 1 AND 64 OR seed_ref GLOB '*[^A-Za-z0-9._-]*' OR typeof(seed_digest)!='text' OR octet_length(seed_digest)!=64 OR seed_digest GLOB '*[^0-9a-f]*' OR typeof(last_step)!='integer' OR last_step<0 OR typeof(consumption_json)!='text' OR octet_length(consumption_json) NOT BETWEEN 1 AND 2048",
        "SELECT count(*) FROM totp_factor WHERE ((enrollment_json IS NULL)+(intent_receipt IS NULL)+(outcome_receipt IS NULL)+(confirmed_at IS NULL)) NOT IN (0,4) OR (enrollment_json IS NOT NULL AND (typeof(enrollment_json)!='text' OR octet_length(enrollment_json) NOT BETWEEN 1 AND 2048)) OR (intent_receipt IS NOT NULL AND (typeof(intent_receipt)!='text' OR octet_length(intent_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND (typeof(outcome_receipt)!='text' OR octet_length(outcome_receipt) NOT BETWEEN 1 AND 8192)) OR (confirmed_at IS NOT NULL AND (typeof(confirmed_at)!='integer' OR confirmed_at<0))",
        "SELECT count(*) FROM totp_pending WHERE id!=1 OR typeof(operation_json)!='text' OR octet_length(operation_json) NOT BETWEEN 1 AND 2048 OR (intent_receipt IS NOT NULL AND (typeof(intent_receipt)!='text' OR octet_length(intent_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND (typeof(outcome_receipt)!='text' OR octet_length(outcome_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND intent_receipt IS NULL)",
        "SELECT count(*) FROM totp_grants WHERE typeof(session_id)!='text' OR octet_length(session_id) NOT BETWEEN 1 AND 32 OR typeof(operation_json)!='text' OR octet_length(operation_json) NOT BETWEEN 1 AND 2048 OR typeof(intent_receipt)!='text' OR octet_length(intent_receipt) NOT BETWEEN 1 AND 8192 OR typeof(outcome_receipt)!='text' OR octet_length(outcome_receipt) NOT BETWEEN 1 AND 8192 OR typeof(activated_at)!='integer' OR activated_at<0",
    ] {
        let invalid: i64 = db.query_row(query, [], |r| r.get(0)).map_err(unavailable)?;
        if invalid != 0 {
            return Err(SessionError::CorruptStore);
        }
    }
    verification_rate::validate(db, Scope::Totp)?;
    validate::all(db)
}
