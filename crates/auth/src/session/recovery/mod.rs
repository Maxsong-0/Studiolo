//! 恢复码生命周期与实际因子替换；没有可缓存的恢复权限或远端秘密装配。

mod flow;
mod model;
mod mutations;
mod originals;
mod pipeline;
mod prepared;
mod replacement;
mod rotation;
mod runtime;
pub(super) mod schema;
mod secrets;
mod store;

pub use prepared::PreparedTotpFactor;
pub use secrets::{RecoveryCode, RecoveryCodes};

use super::{SessionError, SessionManager};
use std::sync::Arc;

/// 仅通过可信装配取得的恢复 Module；不读取秘密文件、不设置 Cookie。
pub struct Recovery {
    sessions: Arc<SessionManager>,
}

impl Recovery {
    /// 只打开已经显式升级的 v4；不生成恢复码或改变因子。
    pub fn attach(sessions: Arc<SessionManager>) -> Result<Self, SessionError> {
        sessions.database.transaction(|db| {
            if super::schema::version(db)? != 4 {
                return Err(SessionError::MigrationRequired);
            }
            Ok(())
        })?;
        Ok(Self { sessions })
    }
}

/// 固定的最近恢复候选消费锚点；旧 v2/v3 没有此状态。
pub(super) fn high_step(db: &rusqlite::Connection) -> Result<i64, SessionError> {
    if super::schema::version(db)? < 4 {
        return Ok(-1);
    }
    Ok(store::control(db)?.high_step)
}

pub(super) fn has_pending(db: &rusqlite::Connection) -> Result<bool, SessionError> {
    Ok(super::schema::version(db)? >= 4 && store::pending(db)?.is_some())
}

pub(super) fn blocks_retention(db: &rusqlite::Connection, id: &str) -> Result<bool, SessionError> {
    if super::schema::version(db)? < 4 {
        return Ok(false);
    }
    Ok(store::pending(db)?.is_some_and(|p| p.operation.session_id == id))
}
