//! 预扣预算和单调序号先独立提交；最终执行重验同一会话、因子及期限。

use super::super::{
    SessionError, TotpRequest,
    denials::Denial,
    recent, state, totp,
    verification_rate::{self, Scope},
};
use super::{
    Recovery,
    model::{Kind, Operation},
    pipeline, store,
};
use rusqlite::Connection;

pub(super) struct Ticket {
    pub sequence: i64,
    pub epoch: i64,
    pub session_id: String,
    pub source_enrollment: i64,
    pub source_digest: String,
    pub deadline: i64,
    pub absolute: i64,
}

impl Recovery {
    pub(super) fn prepare(
        &self,
        request: &TotpRequest<'_>,
        kind: Kind,
    ) -> Result<Ticket, SessionError> {
        self.sessions.run(|db, observed| {
            let row = match self.sessions.authorized(
                db,
                observed,
                request.identity,
                request.token,
                Some(request.csrf),
            )? {
                Ok(row) => row,
                Err(error) => return Ok(Err(error)),
            };
            pipeline::release_invalid(db, observed)?;
            if store::pending(db)?.is_some() || totp::store::pending(db)?.is_some() {
                return expected(db, SessionError::OperationPending);
            }
            let Some(factor) = totp::store::factor(db)? else {
                return expected(db, SessionError::FactorUnavailable);
            };
            let Some(enrollment) = factor.enrollment else {
                return expected(db, SessionError::FactorUnavailable);
            };
            if kind == Kind::Rotate {
                if let Err(error) = require_l3(db, observed, row.info.id.as_str())? {
                    return expected(db, error);
                }
            } else if let Err(error) =
                verification_rate::reserve(db, Scope::Recovery, observed.now, Some(request.source))?
            {
                return expected(db, error);
            }
            let absolute = row
                .info
                .absolute_expires_at_ms
                .min(state::access_expiry(request.identity)?);
            let Some(deadline) = observed.now.checked_add(300_000) else {
                return expected(db, SessionError::ClockRollback);
            };
            let sequence = match store::reserve(db) {
                Ok(value) => value,
                Err(error) => return expected(db, error),
            };
            Ok(Ok(Ticket {
                sequence,
                epoch: observed.epoch,
                session_id: row.info.id.as_str().to_owned(),
                source_enrollment: enrollment.operation.sequence,
                source_digest: factor.seed_digest,
                deadline: deadline.min(absolute),
                absolute,
            }))
        })
    }

    pub(super) fn recheck(
        &self,
        db: &Connection,
        observed: &state::Observed,
        request: &TotpRequest<'_>,
        ticket: &Ticket,
        deadline: i64,
    ) -> Result<Result<(), SessionError>, SessionError> {
        if observed.epoch != ticket.epoch || observed.now >= deadline {
            return expected(db, SessionError::Rejected);
        }
        let row = match self.sessions.authorized(
            db,
            observed,
            request.identity,
            request.token,
            Some(request.csrf),
        )? {
            Ok(row) => row,
            Err(error) => return Ok(Err(error)),
        };
        let Some(factor) = totp::store::factor(db)? else {
            return expected(db, SessionError::FactorUnavailable);
        };
        if row.info.id.as_str() != ticket.session_id
            || factor.seed_digest != ticket.source_digest
            || factor
                .enrollment
                .is_none_or(|p| p.operation.sequence != ticket.source_enrollment)
        {
            return expected(db, SessionError::Rejected);
        }
        if totp::store::pending(db)?.is_some() {
            return expected(db, SessionError::OperationPending);
        }
        Ok(Ok(()))
    }
}

impl Ticket {
    pub fn operation(&self, kind: Kind, now: i64) -> Result<Operation, SessionError> {
        let deadline = now
            .checked_add(300_000)
            .ok_or(SessionError::ClockRollback)?
            .min(self.absolute);
        Ok(Operation {
            sequence: self.sequence,
            kind,
            session_id: self.session_id.clone(),
            epoch: self.epoch,
            generation: self.sequence,
            source_enrollment: self.source_enrollment,
            source_digest: self.source_digest.clone(),
            verified_at: now,
            deadline,
            hashes: Vec::new(),
            slot: None,
            target: None,
        })
    }
}

pub(super) fn require_l3(
    db: &Connection,
    observed: &state::Observed,
    id: &str,
) -> Result<Result<(), SessionError>, SessionError> {
    if recent::authentication(db, observed, id)?.is_none() {
        return Ok(Err(SessionError::ReauthenticationRequired));
    }
    Ok(Ok(()))
}

pub(super) fn expected<T>(
    db: &Connection,
    error: SessionError,
) -> Result<Result<T, SessionError>, SessionError> {
    match error {
        SessionError::StorageUnavailable
        | SessionError::CorruptStore
        | SessionError::UnsafeStorage
        | SessionError::UnsupportedSchema
        | SessionError::UnsupportedPlatform => Err(error),
        _ => state::deny(db, Denial::Issue, error),
    }
}

pub(super) fn stageable(
    db: &Connection,
    ticket: &Ticket,
) -> Result<Result<(), SessionError>, SessionError> {
    if store::pending(db)?.is_some() {
        return expected(db, SessionError::OperationPending);
    }
    // 随机源等待期间有另一调用取得序号时，旧调用不得倒序发布决定。
    if store::next(db)? != ticket.sequence + 1 {
        return expected(db, SessionError::Rejected);
    }
    Ok(Ok(()))
}
