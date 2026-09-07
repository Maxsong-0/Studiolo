//! 近期认证的唯一私有期限规则；调用方在同一事务内完成会话及当前因子校验。

use super::{RecentAuthentication, SessionError, state::Observed, totp};
use rusqlite::Connection;

pub(super) fn authentication(
    db: &Connection,
    observed: &Observed,
    session: &str,
) -> Result<Option<RecentAuthentication>, SessionError> {
    let Some(proof) = totp::store::grant(db, session)? else {
        return Ok(None);
    };
    if proof.operation.epoch != observed.epoch || observed.now >= proof.operation.deadline {
        return Ok(None);
    }
    Ok(Some(RecentAuthentication {
        verified_at_ms: proof.operation.verified_at,
        expires_at_ms: proof.operation.deadline,
    }))
}
