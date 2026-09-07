//! 有界会话行读取；摘要只留在私有实现。

use super::{SessionError, SessionId, SessionInfo, SessionState, database::unavailable};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) struct SessionRow {
    pub info: SessionInfo,
    pub token_digest: Vec<u8>,
    pub csrf_digest: Vec<u8>,
    pub epoch: i64,
    pub grant_id: String,
    pub grant_receipt: Option<String>,
    pub activated_at: Option<i64>,
}

const SELECT: &str = "SELECT id,state,created,last_seen,absolute_expiry,token_digest,csrf_digest,epoch,grant_id,grant_receipt,activated_at FROM sessions";

fn read(row: &rusqlite::Row<'_>) -> Result<SessionRow, rusqlite::Error> {
    let state: String = row.get(1)?;
    let state = SessionState::parse(&state).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(SessionRow {
        info: SessionInfo {
            id: SessionId(row.get(0)?),
            state,
            created_at_ms: row.get(2)?,
            last_seen_at_ms: row.get(3)?,
            absolute_expires_at_ms: row.get(4)?,
        },
        token_digest: row.get(5)?,
        csrf_digest: row.get(6)?,
        epoch: row.get(7)?,
        grant_id: row.get(8)?,
        grant_receipt: row.get(9)?,
        activated_at: row.get(10)?,
    })
}

pub(super) fn by_digest(
    db: &Connection,
    digest: &[u8; 32],
) -> Result<Option<SessionRow>, SessionError> {
    db.query_row(
        &format!("{SELECT} WHERE token_digest=?1"),
        params![digest.as_slice()],
        read,
    )
    .optional()
    .map_err(unavailable)
}

pub(super) fn by_id(db: &Connection, id: &SessionId) -> Result<Option<SessionRow>, SessionError> {
    db.query_row(&format!("{SELECT} WHERE id=?1"), [id.as_str()], read)
        .optional()
        .map_err(unavailable)
}

pub(super) fn all(db: &Connection) -> Result<Vec<SessionRow>, SessionError> {
    let mut statement = db
        .prepare(&format!("{SELECT} ORDER BY created,id"))
        .map_err(unavailable)?;
    statement
        .query_map([], read)
        .map_err(unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unavailable)
}

pub(super) fn validate(db: &Connection, config: &str) -> Result<(), SessionError> {
    super::session_validation::sessions(db, config)
}
