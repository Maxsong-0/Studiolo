//! 先持久受理预算，再消费验证码；固定决定交付后仅原调用有最终激活路径。

use super::super::{
    SessionError,
    denials::Denial,
    recovery, state,
    verification_rate::{self, Scope},
};
use super::{
    Context, RecentAuthentication, TotpCode, TotpRequest, crypto,
    model::{Factor, Kind, Operation},
    pipeline, store,
};
use rusqlite::Connection;

struct Ticket {
    epoch: i64,
    session_id: Option<String>,
    deadline: i64,
    absolute: i64,
}

impl Context {
    pub(super) fn factor(
        &self,
        db: &Connection,
        enrolling: bool,
    ) -> Result<Option<Factor>, SessionError> {
        let factor = store::factor(db)?;
        if let Some(factor) = &factor {
            if factor.seed_ref != self.seed.reference()
                || factor.seed_digest != self.seed.fingerprint()
            {
                return Err(SessionError::Configuration);
            }
            if enrolling && factor.enrollment.is_some() {
                return Err(SessionError::AlreadyEnrolled);
            }
        }
        if !enrolling && factor.as_ref().is_none_or(|f| f.enrollment.is_none()) {
            return Err(SessionError::FactorUnavailable);
        }
        Ok(factor)
    }

    pub(super) fn execute(
        &self,
        request: Option<&TotpRequest<'_>>,
        code: &TotpCode,
    ) -> Result<RecentAuthentication, SessionError> {
        let ticket = self.prepare(request)?;
        let operation = self.sessions.run(|db, observed| {
            if let Err(error) = self.recheck(db, observed, &ticket, ticket.deadline, request)? {
                return Ok(Err(error));
            }
            let factor = match self.factor(db, request.is_none()) {
                Ok(value) => value,
                Err(error) => return expected(db, error),
            };
            if store::pending(db)?.is_some() {
                return expected(db, SessionError::OperationPending);
            }
            let last_step = factor
                .as_ref()
                .map_or(-1, |f| f.last_step)
                .max(recovery::high_step(db)?);
            let Some(step) = crypto::matching_step(&self.seed, code, observed.now, last_step)
            else {
                return expected(db, SessionError::Rejected);
            };
            let deadline = match observed.now.checked_add(300_000) {
                Some(deadline) => deadline.min(ticket.absolute),
                None => return expected(db, SessionError::ClockRollback),
            };
            let sequence = match store::reserve(db) {
                Ok(value) => value,
                Err(SessionError::Capacity) => return expected(db, SessionError::Capacity),
                Err(error) => return Err(error),
            };
            let operation = Operation {
                sequence,
                kind: if request.is_some() {
                    Kind::StepUp
                } else {
                    Kind::Enroll
                },
                session_id: ticket.session_id.clone(),
                epoch: ticket.epoch,
                seed_ref: self.seed.reference().to_owned(),
                seed_digest: self.seed.fingerprint(),
                step,
                verified_at: observed.now,
                deadline,
            };
            store::consume(db, &operation)?;
            super::schema::validate(db)?;
            Ok(Ok(operation))
        })?;
        if let Err(error) = self.deliver(Some(&operation)) {
            return self.sessions.run(|db, _| expected(db, error));
        }
        self.sessions.run(|db, observed| {
            if let Err(error) = self.recheck(db, observed, &ticket, operation.deadline, request)? {
                return Ok(Err(error));
            }
            if let Err(error) = self.factor(db, request.is_none()) {
                return expected(db, error);
            }
            let Some(pending) = store::pending(db)? else {
                return expected(db, SessionError::Rejected);
            };
            if pending.operation != operation {
                return expected(db, SessionError::Rejected);
            }
            if pending.intent_receipt.is_none() || pending.outcome_receipt.is_none() {
                return expected(db, SessionError::AuditUnavailable);
            }
            store::activate(db, &pending, observed.now)?;
            super::schema::validate(db)?;
            Ok(Ok(RecentAuthentication {
                verified_at_ms: operation.verified_at,
                expires_at_ms: operation.deadline,
            }))
        })
    }

    fn prepare(&self, request: Option<&TotpRequest<'_>>) -> Result<Ticket, SessionError> {
        self.sessions.run(|db, observed| {
            if recovery::has_pending(db)? {
                return expected(db, SessionError::OperationPending);
            }
            let mut deadline = match observed.now.checked_add(300_000) {
                Some(value) => value,
                None => return expected(db, SessionError::ClockRollback),
            };
            let mut absolute = i64::MAX;
            let session_id = if let Some(request) = request {
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
                absolute = row
                    .info
                    .absolute_expires_at_ms
                    .min(state::access_expiry(request.identity)?);
                deadline = deadline.min(absolute);
                Some(row.info.id.as_str().to_owned())
            } else {
                None
            };
            if let Err(error) = self.factor(db, request.is_none()) {
                return expected(db, error);
            }
            pipeline::release_invalid(db, observed)?;
            if store::pending(db)?.is_some() {
                return expected(db, SessionError::OperationPending);
            }
            if let Err(error) = verification_rate::reserve(
                db,
                Scope::Totp,
                observed.now,
                request.map(|r| r.source),
            )? {
                return expected(db, error);
            }
            Ok(Ok(Ticket {
                epoch: observed.epoch,
                session_id,
                deadline,
                absolute,
            }))
        })
    }

    fn recheck(
        &self,
        db: &Connection,
        observed: &state::Observed,
        ticket: &Ticket,
        deadline: i64,
        request: Option<&TotpRequest<'_>>,
    ) -> Result<Result<(), SessionError>, SessionError> {
        if recovery::has_pending(db)? {
            return expected(db, SessionError::OperationPending);
        }
        if observed.epoch != ticket.epoch || observed.now >= deadline {
            return expected(db, SessionError::Rejected);
        }
        if let Some(request) = request {
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
            if Some(row.info.id.as_str()) != ticket.session_id.as_deref() {
                return expected(db, SessionError::Rejected);
            }
        }
        Ok(Ok(()))
    }
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
