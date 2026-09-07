//! 一次交付新批次；任何失败均不重读原码，也不延长原 L3。

use super::super::{SessionError, TotpRequest, credential_store, schema as auth_schema};
use super::{
    Recovery, RecoveryCodes,
    flow::{self, expected},
    model::Kind,
    mutations, schema, secrets, store,
};

impl Recovery {
    /// 以当前真实 L3 换发十枚恢复码。成功后整批旧码失效，只有本调用取得新原码。
    pub fn rotate_codes(&self, request: &TotpRequest<'_>) -> Result<RecoveryCodes, SessionError> {
        let ticket = self.prepare(request, Kind::Rotate)?;
        let store_id = self.sessions.database.transaction(auth_schema::store_id)?;
        let generated =
            match secrets::generate(self.sessions.random.as_ref(), &store_id, ticket.sequence) {
                Ok(value) => value,
                Err(error) => return self.sessions.run(|db, _| expected(db, error)),
            };
        let operation = self.sessions.run(|db, observed| {
            if let Err(error) = self.recheck(db, observed, request, &ticket, ticket.deadline)? {
                return Ok(Err(error));
            }
            if let Err(error) = flow::require_l3(db, observed, &ticket.session_id)? {
                return expected(db, error);
            }
            if let Err(error) = flow::stageable(db, &ticket)? {
                return Ok(Err(error));
            }
            for (token, csrf) in &generated.seed_digests {
                if credential_store::collided(db, token, csrf, token, csrf)? {
                    return expected(db, SessionError::RandomCollision);
                }
            }
            let mut operation = match ticket.operation(Kind::Rotate, observed.now) {
                Ok(op) => op,
                Err(error) => return expected(db, error),
            };
            operation.hashes.clone_from(&generated.digests);
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
            if let Err(error) = flow::require_l3(db, observed, &ticket.session_id)? {
                return expected(db, error);
            }
            let Some(pending) = store::pending(db)? else {
                return expected(db, SessionError::Rejected);
            };
            if pending.operation != operation {
                return expected(db, SessionError::Rejected);
            }
            mutations::activate(db, &pending, observed.now)?;
            schema::validate(db)?;
            Ok(Ok(()))
        })?;
        Ok(generated.codes)
    }
}
