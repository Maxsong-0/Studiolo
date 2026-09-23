//! 任何真实的外部 I/O 之前，先在 SQLite 中完成预留；结果未知的调用永不重放。
use crate::{
    CallId,
    relay_types::{RelayError, RelayGrant},
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub(crate) fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) struct Store(Mutex<Connection>);
pub(crate) enum Admission {
    Execute,
    Complete {
        raw: String,
        digest: String,
        expires: i64,
    },
}
impl Store {
    pub(crate) fn open(path: &Path) -> Result<Self, RelayError> {
        let parent = path.parent().ok_or(RelayError::Storage)?;
        let metadata = fs::symlink_metadata(parent).map_err(|_| RelayError::Storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(RelayError::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            if metadata.mode() & 0o077 != 0 {
                return Err(RelayError::Storage);
            }
            if !path.exists() {
                File::options()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                    .map_err(|_| RelayError::Storage)?;
            }
            let metadata = fs::symlink_metadata(path).map_err(|_| RelayError::Storage)?;
            if !metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
                return Err(RelayError::Storage);
            }
        }
        let connection = Connection::open(path).map_err(|_| RelayError::Storage)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| RelayError::Storage)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS relay_settings(id INTEGER PRIMARY KEY CHECK(id=1), frozen INTEGER NOT NULL);
            INSERT OR IGNORE INTO relay_settings VALUES(1,1);
            CREATE TABLE IF NOT EXISTS relay_grants(ticket_hash TEXT PRIMARY KEY, grant_json TEXT NOT NULL,
                attempt TEXT UNIQUE NOT NULL, spent INTEGER NOT NULL DEFAULT 0, calls INTEGER NOT NULL DEFAULT 0, tools INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS relay_calls(attempt TEXT NOT NULL, call_id TEXT NOT NULL,
                request_hash TEXT NOT NULL, state TEXT NOT NULL, raw TEXT, digest TEXT, expires INTEGER,
                PRIMARY KEY(attempt,call_id));").map_err(|_| RelayError::Storage)?;
        // 每次进程启动都失败关闭（置为冻结）；只有显式的可信操作才能解冻准入。
        connection
            .execute("UPDATE relay_settings SET frozen=1 WHERE id=1", [])
            .map_err(|_| RelayError::Storage)?;
        Ok(Self(Mutex::new(connection)))
    }
    fn lock(&self) -> Result<MutexGuard<'_, Connection>, RelayError> {
        self.0.lock().map_err(|_| RelayError::Storage)
    }
    pub(crate) fn freeze(&self, freeze: bool) -> Result<(), RelayError> {
        self.lock()?
            .execute("UPDATE relay_settings SET frozen=?1 WHERE id=1", [freeze])
            .map_err(|_| RelayError::Storage)?;
        Ok(())
    }
    pub(crate) fn frozen(&self) -> Result<bool, RelayError> {
        self.lock()?
            .query_row("SELECT frozen FROM relay_settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|_| RelayError::Storage)
    }
    pub(crate) fn grant(&self, token: &str, grant: &RelayGrant) -> Result<(), RelayError> {
        let json = serde_json::to_string(grant).map_err(|_| RelayError::Storage)?;
        self.lock()?
            .execute(
                "INSERT INTO relay_grants(ticket_hash,grant_json,attempt) VALUES(?1,?2,?3)",
                params![
                    hash(token.as_bytes()),
                    json,
                    grant.capability.attempt_id().to_string()
                ],
            )
            .map_err(|_| RelayError::Conflict)?;
        Ok(())
    }
    pub(crate) fn authenticate(&self, token: &str) -> Result<RelayGrant, RelayError> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RelayError::Denied);
        }
        let json: String = self
            .lock()?
            .query_row(
                "SELECT grant_json FROM relay_grants WHERE ticket_hash=?1",
                [hash(token.as_bytes())],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| RelayError::Storage)?
            .ok_or(RelayError::Denied)?;
        serde_json::from_str(&json).map_err(|_| RelayError::Storage)
    }
    pub(crate) fn reserve(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        request: &str,
    ) -> Result<Admission, RelayError> {
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| RelayError::Storage)?;
        let frozen: bool = tx
            .query_row("SELECT frozen FROM relay_settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|_| RelayError::Storage)?;
        if frozen {
            return Err(RelayError::Frozen);
        }
        let attempt = grant.capability.attempt_id().to_string();
        let existing = tx.query_row("SELECT request_hash,state,raw,digest,expires FROM relay_calls WHERE attempt=?1 AND call_id=?2",
            params![attempt, call.as_str()], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?,
                r.get::<_,Option<String>>(2)?, r.get::<_,Option<String>>(3)?, r.get::<_,Option<i64>>(4)?)))
            .optional().map_err(|_| RelayError::Storage)?;
        if let Some((digest, state, raw, sha, expires)) = existing {
            if digest != request {
                return Err(RelayError::Conflict);
            }
            return if state == "complete" {
                Ok(Admission::Complete {
                    raw: raw.ok_or(RelayError::Storage)?,
                    digest: sha.ok_or(RelayError::Storage)?,
                    expires: expires.ok_or(RelayError::Storage)?,
                })
            } else {
                Err(RelayError::Indeterminate)
            };
        }
        let unresolved: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM relay_calls WHERE attempt=?1 AND state!='complete')",
                [&attempt],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::Storage)?;
        if unresolved {
            return Err(RelayError::Indeterminate);
        }
        let ceiling =
            i64::try_from(grant.limits.budget_microusd).map_err(|_| RelayError::Budget)?;
        let cost =
            i64::try_from(grant.limits.reservation_microusd).map_err(|_| RelayError::Budget)?;
        if cost <= 0 || cost > ceiling {
            return Err(RelayError::Budget);
        }
        let updated = tx.execute("UPDATE relay_grants SET spent=spent+?1,calls=calls+1 WHERE attempt=?2 AND spent<=?3-?1 AND calls<?4",
            params![cost, attempt, ceiling, grant.limits.calls]).map_err(|_| RelayError::Storage)?;
        if updated != 1 {
            return Err(RelayError::Budget);
        }
        tx.execute(
            "INSERT INTO relay_calls(attempt,call_id,request_hash,state) VALUES(?1,?2,?3,'intent')",
            params![attempt, call.as_str(), request],
        )
        .map_err(|_| RelayError::Storage)?;
        tx.commit().map_err(|_| RelayError::Storage)?;
        Ok(Admission::Execute)
    }
    pub(crate) fn tools(&self, grant: &RelayGrant) -> Result<u32, RelayError> {
        self.lock()?
            .query_row(
                "SELECT tools FROM relay_grants WHERE attempt=?1",
                [grant.capability.attempt_id().to_string()],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::Storage)
    }
    pub(crate) fn accept_tools(&self, grant: &RelayGrant, count: u32) -> Result<(), RelayError> {
        if count > grant.limits.max_tools {
            return Err(RelayError::Budget);
        }
        let updated = self
            .lock()?
            .execute(
                "UPDATE relay_grants SET tools=tools+?1 WHERE attempt=?2 AND tools<=?3-?1",
                params![
                    count,
                    grant.capability.attempt_id().to_string(),
                    grant.limits.max_tools
                ],
            )
            .map_err(|_| RelayError::Storage)?;
        if updated != 1 {
            return Err(RelayError::Budget);
        }
        Ok(())
    }
    pub(crate) fn mark(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        state: &str,
    ) -> Result<(), RelayError> {
        self.lock()?.execute("UPDATE relay_calls SET state=?1 WHERE attempt=?2 AND call_id=?3 AND state!='complete'",
            params![state, grant.capability.attempt_id().to_string(), call.as_str()]).map_err(|_| RelayError::Storage)?;
        Ok(())
    }
    pub(crate) fn complete(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        raw: &str,
        digest: &str,
        expires: i64,
    ) -> Result<(), RelayError> {
        self.lock()?.execute("UPDATE relay_calls SET state='complete',raw=?1,digest=?2,expires=?3 WHERE attempt=?4 AND call_id=?5",
            params![raw,digest,expires,grant.capability.attempt_id().to_string(),call.as_str()]).map_err(|_| RelayError::Storage)?;
        Ok(())
    }
}
