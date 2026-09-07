//! 恢复只执行实际因子替换；消费先提交，成功切换与全部吊销同事务。

use super::super::{
    SessionError, Totp, TotpCode, TotpRequest, revocation, schema as auth_schema,
    secrets as session_secrets, totp,
};
use super::{
    PreparedTotpFactor, Recovery, RecoveryCode,
    flow::{self, expected},
    model::Kind,
    mutations, schema, store,
};

impl Recovery {
    /// 真实 Access、L2、CSRF 加一次恢复码及新因子验证码，原子替换登记并吊销全部会话。
    ///
    /// 新种子须已由可信装配保存并交付。失败可能消费码；补记不会重试切换。
    /// 返回值只是新因子句柄，调用者必须重新登录并重新提权。
    pub fn replace_factor(
        &self,
        request: &TotpRequest<'_>,
        code: RecoveryCode,
        prepared: &PreparedTotpFactor,
        otp: TotpCode,
    ) -> Result<Totp, SessionError> {
        let ticket = self.prepare(request, Kind::Replace)?;
        let operation = self.sessions.run(|db, observed| {
            if let Err(error) = self.recheck(db, observed, request, &ticket, ticket.deadline)? {
                return Ok(Err(error));
            }
            if let Err(error) = flow::stageable(db, &ticket)? {
                return Ok(Err(error));
            }
            if let Err(error) = prepared.check(db) {
                return expected(db, error);
            }
            let control = store::control(db)?;
            if control
                .current
                .as_ref()
                .is_none_or(|p| p.operation.generation != code.generation())
            {
                return expected(db, SessionError::Rejected);
            }
            let digest = code.digest(&auth_schema::store_id(db)?);
            let valid = store::codes(db)?.into_iter().any(|stored| {
                stored.generation == code.generation()
                    && stored.slot == code.slot()
                    && stored.consumption.is_none()
                    && session_secrets::equal(stored.digest.as_bytes(), digest.as_bytes())
            });
            if !valid {
                return expected(db, SessionError::Rejected);
            }
            let factor = totp::store::factor(db)?.ok_or(SessionError::CorruptStore)?;
            let Some(step) = totp::crypto::matching_step(
                &prepared.context.seed,
                &otp,
                observed.now,
                factor.last_step.max(control.high_step),
            ) else {
                return expected(db, SessionError::Rejected);
            };
            let Some(epoch) = observed.epoch.checked_add(1) else {
                return expected(db, SessionError::Capacity);
            };
            let factor_sequence = match totp::store::reserve(db) {
                Ok(value) => value,
                Err(error) => return expected(db, error),
            };
            let mut operation = match ticket.operation(Kind::Replace, observed.now) {
                Ok(value) => value,
                Err(error) => return expected(db, error),
            };
            operation.generation = code.generation();
            operation.slot = Some(code.slot());
            operation.target = Some(totp::model::Operation {
                sequence: factor_sequence,
                kind: totp::model::Kind::Recovered {
                    operation: operation.sequence,
                },
                session_id: None,
                epoch,
                seed_ref: prepared.context.seed.reference().to_owned(),
                seed_digest: prepared.context.seed.fingerprint(),
                step,
                verified_at: operation.verified_at,
                deadline: operation.deadline,
            });
            mutations::stage(db, &operation)?;
            schema::validate(db)?;
            Ok(Ok(operation))
        })?;
        if let Err(error) = self.deliver(Some(&operation)) {
            return self.sessions.run(|db, _| expected(db, error));
        }
        self.sessions.run(|db, observed| {
            if let Err(error) = self.recheck(db, observed, request, &ticket, operation.deadline)? {
                return Ok(Err(error));
            }
            if let Err(error) = prepared.check(db) {
                return expected(db, error);
            }
            let Some(pending) = store::pending(db)? else {
                return expected(db, SessionError::Rejected);
            };
            if pending.operation != operation {
                return expected(db, SessionError::Rejected);
            }
            let proof = store::proof(&pending, observed.now)?;
            if observed.epoch.checked_add(1).is_none() {
                return expected(db, SessionError::Capacity);
            }
            // 此后任何失败都回滚整个切换；此前持久消费的码仍不复活。
            revocation::all(db, observed)?;
            totp::store::replace(
                db,
                &totp::model::Proof {
                    operation: operation.target.clone().ok_or(SessionError::CorruptStore)?,
                    intent_receipt: proof.intent_receipt,
                    outcome_receipt: proof.outcome_receipt,
                    activated_at: observed.now,
                },
            )?;
            mutations::activate(db, &pending, observed.now)?;
            auth_schema::validate(db, &self.sessions.config.encoded()?)?;
            Ok(Ok(()))
        })?;
        let _ = self.sessions.flush_audit(256);
        Ok(Totp {
            context: prepared.context.clone(),
        })
    }
}
