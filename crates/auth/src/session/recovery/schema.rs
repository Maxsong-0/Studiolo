//! v4 只追加四张固定表；原始值有界检查先于任何 JSON 解码。

#[path = "validate.rs"]
mod validate;

use crate::session::{
    SessionError,
    database::unavailable,
    verification_rate::{self, Scope},
};
use rusqlite::Connection;

const CONTROL: &str = "CREATE TABLE recovery_control (
 id INTEGER PRIMARY KEY CHECK(id=1), next_operation INTEGER NOT NULL CHECK(next_operation>0),
 current_json TEXT, change_json TEXT, high_step INTEGER NOT NULL,
 consumption_json TEXT, used_mask INTEGER NOT NULL
) STRICT";
const CODES: &str = "CREATE TABLE recovery_codes (
 generation INTEGER NOT NULL, slot INTEGER NOT NULL, digest TEXT NOT NULL,
 consumption_json TEXT, intent_receipt TEXT, outcome_receipt TEXT,
 PRIMARY KEY(generation,slot)
) STRICT";
const PENDING: &str = "CREATE TABLE recovery_pending (
 id INTEGER PRIMARY KEY CHECK(id=1), operation_json TEXT NOT NULL,
 intent_receipt TEXT, outcome_receipt TEXT
) STRICT";
const RATES: &str = "CREATE TABLE recovery_rates (
 bucket TEXT PRIMARY KEY, window_started INTEGER NOT NULL, attempts INTEGER NOT NULL
) STRICT";

pub(in crate::session) const OBJECTS: &[(&str, Option<&str>)] = &[
    ("recovery_control", Some(CONTROL)),
    ("recovery_codes", Some(CODES)),
    ("recovery_pending", Some(PENDING)),
    ("recovery_rates", Some(RATES)),
    ("sqlite_autoindex_recovery_codes_1", None),
    ("sqlite_autoindex_recovery_rates_1", None),
];

pub(in crate::session) fn initialize(db: &Connection) -> Result<(), SessionError> {
    db.execute_batch(&format!("{CONTROL};{CODES};{PENDING};{RATES};INSERT INTO recovery_control VALUES(1,1,NULL,NULL,-1,NULL,0);PRAGMA user_version=4;"))
        .map_err(unavailable)
}

pub(in crate::session) fn validate(db: &Connection) -> Result<(), SessionError> {
    for (table, maximum, required) in [
        ("recovery_control", 1_i64, true),
        ("recovery_codes", 20, false),
        ("recovery_pending", 1, false),
        ("recovery_rates", 66, false),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(unavailable)?;
        if count > maximum || (required && count != 1) {
            return Err(SessionError::CorruptStore);
        }
    }
    for query in [
        "SELECT count(*) FROM recovery_control WHERE id!=1 OR typeof(next_operation)!='integer' OR next_operation<=0 OR typeof(high_step)!='integer' OR high_step< -1 OR typeof(used_mask)!='integer' OR used_mask NOT BETWEEN 0 AND 1023 OR ((consumption_json IS NULL)!=(high_step=-1)) OR (current_json IS NOT NULL AND (typeof(current_json)!='text' OR octet_length(current_json) NOT BETWEEN 1 AND 65536)) OR (change_json IS NOT NULL AND (typeof(change_json)!='text' OR octet_length(change_json) NOT BETWEEN 1 AND 65536)) OR (consumption_json IS NOT NULL AND (typeof(consumption_json)!='text' OR octet_length(consumption_json) NOT BETWEEN 1 AND 4096))",
        "SELECT count(*) FROM recovery_codes WHERE typeof(generation)!='integer' OR generation<=0 OR typeof(slot)!='integer' OR slot NOT BETWEEN 0 AND 9 OR typeof(digest)!='text' OR octet_length(digest)!=64 OR digest GLOB '*[^0-9a-f]*' OR (consumption_json IS NULL AND (intent_receipt IS NOT NULL OR outcome_receipt IS NOT NULL)) OR (consumption_json IS NOT NULL AND (typeof(consumption_json)!='text' OR octet_length(consumption_json) NOT BETWEEN 1 AND 4096)) OR (intent_receipt IS NOT NULL AND (typeof(intent_receipt)!='text' OR octet_length(intent_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND (typeof(outcome_receipt)!='text' OR octet_length(outcome_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND intent_receipt IS NULL)",
        "SELECT count(*) FROM recovery_pending WHERE id!=1 OR typeof(operation_json)!='text' OR octet_length(operation_json) NOT BETWEEN 1 AND 4096 OR (intent_receipt IS NOT NULL AND (typeof(intent_receipt)!='text' OR octet_length(intent_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND (typeof(outcome_receipt)!='text' OR octet_length(outcome_receipt) NOT BETWEEN 1 AND 8192)) OR (outcome_receipt IS NOT NULL AND intent_receipt IS NULL)",
    ] {
        let invalid: i64 = db.query_row(query, [], |r| r.get(0)).map_err(unavailable)?;
        if invalid != 0 {
            return Err(SessionError::CorruptStore);
        }
    }
    verification_rate::validate(db, Scope::Recovery)?;
    validate::all(db)
}
