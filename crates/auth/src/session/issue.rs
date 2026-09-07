//! 新授权只由仍持有本次秘密的调用激活；补记者和重新打开数据库均无激活入口。

use super::{
    IssuedSession, SessionError, SessionManager, SessionState, credential_store,
    database::unavailable, denials::Denial, events::Grant, http_session::IssuePolicy, outbox, rows,
    schema, secrets, state,
};
use crate::AccessIdentity;
use rusqlite::params;
use studiolo_audit_ledger::AuditPhase;

impl SessionManager {
    /// 审计意图、待激活状态、授权决定和最终 CAS 均成功后，才返回一次原始凭证。
    pub fn issue(&self, identity: &AccessIdentity) -> Result<IssuedSession, SessionError> {
        self.issue_with_policy(identity, IssuePolicy::Ordinary)
    }

    pub(super) fn issue_with_policy(
        &self,
        identity: &AccessIdentity,
        policy: IssuePolicy,
    ) -> Result<IssuedSession, SessionError> {
        let (grant, issuance) = self.issue_run(identity, policy, |db, observed| {
            if let Err(error) = self.identity(db, identity, observed.now)? {
                return Ok(Err(error));
            }
            let count: i64 = db
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
                .map_err(unavailable)?;
            if count >= schema::MAX_SESSIONS {
                return state::deny(db, Denial::Capacity, SessionError::Capacity);
            }
            let access_expiry = state::access_expiry(identity)?;
            let absolute = observed
                .now
                .checked_add((self.config.policy.absolute_seconds * 1000) as i64)
                .ok_or(SessionError::ClockRollback)?
                .min(access_expiry);
            // 在任何外部审计前预留持久序号；后续失败可以烧号但绝不复用。
            let issuance = match credential_store::reserve(db) {
                Ok(value) => value,
                Err(SessionError::Capacity) => {
                    return state::deny(db, Denial::Capacity, SessionError::Capacity);
                }
                Err(error) => return Err(error),
            };
            Ok(Ok((
                Grant {
                    store_id: schema::store_id(db)?,
                    id: super::SessionId::parse(format!("v2-{issuance:016x}"))?,
                    epoch: observed.epoch,
                    created: observed.now,
                    absolute,
                    access_expiry,
                },
                issuance,
            )))
        })?;
        let generated = match secrets::generate(self.random.as_ref(), &grant.store_id, issuance) {
            Ok(value) => value,
            Err(error) => {
                return self.issue_run(identity, policy, |db, _| {
                    state::deny(db, Denial::Issue, error)
                });
            }
        };
        self.issue_run(identity, policy, |db, observed| {
            if observed.epoch != grant.epoch || observed.now >= grant.absolute {
                return state::deny(db, Denial::Issue, SessionError::Rejected);
            }
            if collided(db, &generated)? {
                return state::deny(db, Denial::Issue, SessionError::RandomCollision);
            }
            Ok(Ok(()))
        })?;
        let intent = grant.event(AuditPhase::Intent)?;
        let intent_result = self
            .audit
            .append(intent.clone())
            .map_err(|_| SessionError::AuditUnavailable)
            .and_then(|receipt| outbox::receipt(&receipt, &intent));
        if let Err(error) = intent_result {
            return self.issue_run(identity, policy, |db, _| {
                state::deny(db, Denial::Issue, error)
            });
        }
        self.issue_run(identity, policy, |db,observed| {
            if let Err(error)=self.identity(db,identity,observed.now)? { return Ok(Err(error)); }
            if observed.epoch!=grant.epoch || observed.now>=grant.absolute
                || issuance < super::retention::issuance_floor(db)? {
                return state::deny(db,Denial::Issue,SessionError::Rejected);
            }
            let count:i64=db.query_row("SELECT count(*) FROM sessions",[],|r|r.get(0)).map_err(unavailable)?;
            if count>=schema::MAX_SESSIONS { return state::deny(db,Denial::Capacity,SessionError::Capacity); }
            if collided(db,&generated)? { return state::deny(db,Denial::Issue,SessionError::RandomCollision); }
            let outcome=grant.event(AuditPhase::Outcome)?;
            db.execute("INSERT INTO sessions(id,token_digest,csrf_digest,epoch,state,created,last_seen,absolute_expiry,access_expiry,grant_id) VALUES(?1,?2,?3,?4,'pending',?5,?5,?6,?7,?8)",
                params![grant.id.as_str(),generated.token_digest.as_slice(),generated.csrf_digest.as_slice(),grant.epoch,
                    grant.created,grant.absolute,grant.access_expiry,outcome.event_id().as_str()]).map_err(unavailable)?;
            credential_store::insert(db,&grant.id,issuance,&generated.seed_token,&generated.seed_csrf)?;
            outbox::enqueue(db,&outcome,"issue",None)?;
            Ok(Ok(()))
        })?;
        if let Err(error) = self.flush_audit(256) {
            return if error == SessionError::AuditUnavailable {
                self.issue_run(identity, policy, |db, _| {
                    state::deny(db, Denial::Issue, error)
                })
            } else {
                Err(error)
            };
        }
        let session=self.issue_run(identity, policy, |db,observed| {
            if let Err(error)=self.identity(db,identity,observed.now)? { return Ok(Err(error)); }
            let Some(row)=rows::by_id(db,&grant.id)? else {
                // 另一实例可能已把到期/吊销的 pending 连同证据安全回收；不能重新插入。
                return state::deny(db,Denial::Issue,SessionError::Rejected);
            };
            if row.info.state!=SessionState::Pending || row.epoch!=observed.epoch || observed.now>=grant.absolute
                || !secrets::equal(&row.token_digest,&generated.token_digest) {
                return state::deny(db,Denial::Issue,SessionError::Rejected);
            }
            if row.grant_receipt.is_none() { return state::deny(db,Denial::Issue,SessionError::AuditUnavailable); }
            let changed=db.execute("UPDATE sessions SET state='active',last_seen=?1,activated_at=?1 WHERE id=?2 AND state='pending' AND epoch=?3 AND grant_receipt IS NOT NULL AND activated_at IS NULL",
                params![observed.now,grant.id.as_str(),observed.epoch]).map_err(unavailable)?;
            if changed!=1 { return Err(SessionError::CorruptStore); }
            let mut info=row.info; info.state=SessionState::Active; info.last_seen_at_ms=observed.now;
            Ok(Ok(info))
        })?;
        Ok(IssuedSession {
            session,
            token: generated.token,
            csrf: generated.csrf,
        })
    }
}

fn collided(
    db: &rusqlite::Connection,
    generated: &secrets::Generated,
) -> Result<bool, SessionError> {
    let collision:i64=db.query_row("SELECT count(*) FROM sessions WHERE id=?1 OR token_digest IN (?2,?3) OR csrf_digest IN (?4,?5)",
        params![generated.id.as_str(),generated.token_digest.as_slice(),generated.csrf_as_token.as_slice(),
            generated.csrf_digest.as_slice(),generated.token_as_csrf.as_slice()],|r|r.get(0)).map_err(unavailable)?;
    Ok(collision != 0
        || credential_store::collided(
            db,
            &generated.seed_token,
            &generated.seed_token_as_csrf,
            &generated.seed_csrf_as_token,
            &generated.seed_csrf,
        )?)
}
