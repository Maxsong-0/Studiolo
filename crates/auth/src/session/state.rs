//! 所有预期拒绝在提交时间地板和失效状态后返回，不把拒绝分支当事务错误回滚。

use super::{
    CsrfToken, SessionError, SessionManager, SessionState, SessionToken,
    database::unavailable,
    denials::{self, Denial},
    events, outbox,
    rows::{self, SessionRow},
    secrets,
};
use crate::AccessIdentity;
use rusqlite::{Connection, params};
use serde_json::json;
use studiolo_audit_ledger::AuditPhase;

pub(super) struct Observed {
    pub now: i64,
    pub epoch: i64,
}

impl SessionManager {
    pub(super) fn run<T>(
        &self,
        operation: impl FnOnce(&Connection, &Observed) -> Result<Result<T, SessionError>, SessionError>,
    ) -> Result<T, SessionError> {
        self.database.transaction(|db| {
            let (floor, epoch): (i64, i64) = db
                .query_row(
                    "SELECT clock_floor,epoch FROM auth_meta WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(unavailable)?;
            let sampled = self.clock.now_unix_ms();
            let Ok(now) = sampled else {
                return deny(db, Denial::Clock, SessionError::ClockRollback);
            };
            if now < floor || now < 0 {
                return deny(db, Denial::Clock, SessionError::ClockRollback);
            }
            db.execute("UPDATE auth_meta SET clock_floor=?1 WHERE id=1", [now])
                .map_err(unavailable)?;
            expire(db, now, self.config.policy.idle_seconds)?;
            operation(db, &Observed { now, epoch })
        })?
    }

    pub(super) fn identity(
        &self,
        db: &Connection,
        identity: &AccessIdentity,
        now: i64,
    ) -> Result<Result<(), SessionError>, SessionError> {
        if !self.config.owner.matches(identity) {
            return deny(db, Denial::Identity, SessionError::Rejected);
        }
        if access_expiry(identity).is_err() || access_expiry(identity)? <= now {
            return deny(db, Denial::AccessExpired, SessionError::Rejected);
        }
        Ok(Ok(()))
    }

    pub(super) fn authorized(
        &self,
        db: &Connection,
        observed: &Observed,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: Option<&CsrfToken>,
    ) -> Result<Result<SessionRow, SessionError>, SessionError> {
        if let Err(error) = self.identity(db, identity, observed.now)? {
            return Ok(Err(error));
        }
        let digest = secrets::token_digest(token);
        let Some(mut row) = rows::by_digest(db, &digest)? else {
            return deny(db, Denial::Unknown, SessionError::Rejected);
        };
        if matches!(
            row.info.state,
            SessionState::Expired | SessionState::Abandoned
        ) {
            return deny(db, Denial::Expired, SessionError::Rejected);
        }
        if !secrets::equal(&digest, &row.token_digest)
            || row.info.state != SessionState::Active
            || row.epoch != observed.epoch
        {
            return deny(db, Denial::Unknown, SessionError::Rejected);
        }
        if let Some(csrf) = csrf
            && !secrets::equal(&secrets::csrf_digest(csrf), &row.csrf_digest)
        {
            return deny(db, Denial::Csrf, SessionError::Rejected);
        }
        db.execute(
            "UPDATE sessions SET last_seen=?1 WHERE id=?2",
            params![observed.now, row.info.id.as_str()],
        )
        .map_err(unavailable)?;
        row.info.last_seen_at_ms = observed.now;
        Ok(Ok(row))
    }
}

pub(super) fn access_expiry(identity: &AccessIdentity) -> Result<i64, SessionError> {
    identity
        .expires_at()
        .checked_mul(1000)
        .and_then(|v| i64::try_from(v).ok())
        .ok_or(SessionError::Rejected)
}

pub(super) fn deny<T>(
    db: &Connection,
    reason: Denial,
    error: SessionError,
) -> Result<Result<T, SessionError>, SessionError> {
    denials::record(db, reason)?;
    Ok(Err(error))
}

fn expire(db: &Connection, now: i64, idle: u64) -> Result<(), SessionError> {
    for row in rows::all(db)? {
        let pending = row.info.state == SessionState::Pending;
        if !pending && row.info.state != SessionState::Active {
            continue;
        }
        let idle_deadline = row
            .info
            .last_seen_at_ms
            .checked_add((idle * 1000) as i64)
            .ok_or(SessionError::CorruptStore)?;
        let absolute = now >= row.info.absolute_expires_at_ms;
        if !absolute && (pending || now < idle_deadline) {
            continue;
        }
        let state = if pending { "abandoned" } else { "expired" };
        db.execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            params![state, row.info.id.as_str()],
        )
        .map_err(unavailable)?;
        let event = events::event(
            &format!(
                "{}:expire:{}",
                super::schema::store_id(db)?,
                row.info.id.as_str()
            ),
            AuditPhase::Outcome,
            "session.expired",
            json!({"session_id":row.info.id.as_str(),"identity_version":row.epoch,"state":state,
                "reason":if absolute {"absolute"} else {"idle"}}),
        )?;
        outbox::enqueue(db, &event, "expire", None)?;
    }
    Ok(())
}
