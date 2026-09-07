//! 持久会话与独立因子 Interface；不设置 Cookie，不直接执行内容写入。

mod config;
mod credential_store;
mod database;
mod denials;
mod dependencies;
mod event_validation;
mod events;
mod factor_runtime;
mod http_ready;
mod http_session;
mod issue;
mod migration;
mod opening;
mod outbox;
mod recent;
mod recovery;
mod retention;
mod retention_schema;
mod revocation;
mod rows;
mod schema;
mod schema_v1;
mod schema_v2;
mod secrets;
mod session_validation;
mod state;
mod storage;
mod totp;
mod types;
mod validation;
mod verification_rate;

pub use config::{OwnerBinding, SessionConfig, SessionPolicy};
pub use dependencies::{OsSessionRandom, SessionClock, SessionRandom, SystemSessionClock};
pub use factor_runtime::{FactorRuntime, FactorState};
pub use recovery::{PreparedTotpFactor, Recovery, RecoveryCode, RecoveryCodes};
pub use retention::{RetentionReport, RetentionWork};
pub use secrets::{CsrfToken, SessionToken};
pub use totp::{
    RecentAuthentication, SensitiveAction, Totp, TotpCode, TotpEnrollment, TotpEnrollmentState,
    TotpRequest, TotpSeed,
};
pub use types::{IssuedSession, Revocation, SessionError, SessionId, SessionInfo, SessionState};

use crate::AccessIdentity;
use database::Database;
use std::sync::Arc;
use studiolo_audit_ledger::AuditLedger;

/// 独立权威 Auth 存储；多实例经 SQLite 事务协调，审计调用期间不持有 Auth 写锁。
pub struct SessionManager {
    database: Database,
    config: SessionConfig,
    audit: Arc<dyn AuditLedger>,
    clock: Arc<dyn SessionClock>,
    random: Arc<dyn SessionRandom>,
}

impl SessionManager {
    /// 检查本次真实 Access 身份及活跃会话；仅为普通读取，不授予内容 mutation 权限。
    pub fn check(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
    ) -> Result<SessionInfo, SessionError> {
        self.run(|db, observed| {
            Ok(self
                .authorized(db, observed, identity, token, None)?
                .map(|row| row.info))
        })
    }

    /// 经有效会话返回最多 1024 条非秘密元信息；结果不是可缓存的授权凭据。
    pub fn sessions(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
    ) -> Result<Vec<SessionInfo>, SessionError> {
        self.run(|db, observed| {
            if let Err(error) = self.authorized(db, observed, identity, token, None)? {
                return Ok(Err(error));
            }
            Ok(Ok(rows::all(db)?.into_iter().map(|row| row.info).collect()))
        })
    }
}

#[cfg(test)]
#[path = "../../tests/session/mod.rs"]
mod tests;
