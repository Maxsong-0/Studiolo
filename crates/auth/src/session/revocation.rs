//! 吊销与不可变补记同事务提交；审计失败不撤销已经持久生效的撤权。

use super::{
    CsrfToken, Revocation, SessionError, SessionId, SessionManager, SessionState, SessionToken,
    database::unavailable, denials::Denial, events, outbox, rows, schema, state,
};
use crate::AccessIdentity;
use rusqlite::{Connection, params};
use serde_json::json;
use studiolo_audit_ledger::AuditPhase;

impl SessionManager {
    /// 经有效主人会话和该会话的 CSRF 吊销指定会话；不存在或已失效目标不重复产生事件。
    pub fn revoke(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
        target: &SessionId,
    ) -> Result<Revocation, SessionError> {
        let result = self.run(|db, observed| {
            if let Err(error) = self.authorized(db, observed, identity, token, Some(csrf))? {
                return Ok(Err(error));
            }
            Ok(Ok(one(db, target, observed.epoch)?))
        })?;
        // 安全方向已提交；补记失败不改变返回的持久结果，也不重新执行吊销。
        let _ = self.flush_audit(256);
        Ok(result)
    }

    /// 同一事务内验证 Access、会话和 CSRF，并吊销该授权行，不接受调用者指定目标。
    ///
    /// 沿用原吊销事件与补记；审计暂不可用不回滚已经持久提交的撤权。
    pub fn revoke_current(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
    ) -> Result<Revocation, SessionError> {
        let result = self.run(|db, observed| {
            let row = match self.authorized(db, observed, identity, token, Some(csrf))? {
                Ok(row) => row,
                Err(error) => return Ok(Err(error)),
            };
            Ok(Ok(one(db, &row.info.id, observed.epoch)?))
        })?;
        let _ = self.flush_audit(256);
        Ok(result)
    }

    /// 经有效会话和 CSRF 全部吊销，同时推进身份版本；旧签发者不能复活 pending。
    pub fn revoke_all(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
    ) -> Result<Revocation, SessionError> {
        let result = self.run(|db, observed| {
            if let Err(error) = self.authorized(db, observed, identity, token, Some(csrf))? {
                return Ok(Err(error));
            }
            if observed.epoch.checked_add(1).is_none() {
                return state::deny(db, Denial::Capacity, SessionError::Capacity);
            }
            Ok(Ok(all(db, observed)?))
        })?;
        let _ = self.flush_audit(256);
        Ok(result)
    }
}

fn one(db: &Connection, target: &SessionId, epoch: i64) -> Result<Revocation, SessionError> {
    let Some(row) = rows::by_id(db, target)? else {
        return Ok(Revocation { revoked: 0 });
    };
    if !matches!(row.info.state, SessionState::Active | SessionState::Pending) {
        return Ok(Revocation { revoked: 0 });
    }
    db.execute(
        "UPDATE sessions SET state='revoked' WHERE id=?1",
        [target.as_str()],
    )
    .map_err(unavailable)?;
    let id = format!("{}:revoke:{}", schema::store_id(db)?, target.as_str());
    record(db, &id, "one", target.as_str(), 1, epoch)?;
    Ok(Revocation { revoked: 1 })
}

/// 调用方已验权并持有 Auth 写事务；只落本库撤权与原事件，不调用外部审计。
pub(super) fn all(db: &Connection, observed: &state::Observed) -> Result<Revocation, SessionError> {
    if db.is_autocommit() {
        return Err(SessionError::StorageUnavailable);
    }
    let epoch = observed
        .epoch
        .checked_add(1)
        .ok_or(SessionError::Capacity)?;
    let count = db
        .execute(
            "UPDATE sessions SET state='revoked' WHERE state IN ('active','pending')",
            [],
        )
        .map_err(unavailable)?;
    db.execute("UPDATE auth_meta SET epoch=?1 WHERE id=1", params![epoch])
        .map_err(unavailable)?;
    let id = format!("{}:revoke-all:{epoch}", schema::store_id(db)?);
    record(db, &id, "all", "all", count, epoch)?;
    Ok(Revocation { revoked: count })
}

fn record(
    db: &Connection,
    id: &str,
    scope: &str,
    target: &str,
    count: usize,
    epoch: i64,
) -> Result<(), SessionError> {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let event = events::event(
            id,
            phase,
            "session.revoked",
            json!({"scope":scope,"target":target,
            "revoked":count,"identity_version":epoch,"effective":"already_revoked"}),
        )?;
        outbox::enqueue(db, &event, "revoke", None)?;
    }
    Ok(())
}
