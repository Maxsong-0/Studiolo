//! 可信装配预先交付的新因子；只绑定原登记，不暴露可复用的恢复权限。

use super::super::{SessionError, SessionManager, TotpSeed, schema, totp};
use rusqlite::Connection;
use std::sync::Arc;

/// 新秘密必须由可信装配预先安全保存并交付；本类型不写秘密文件或生成二维码。
///
/// 不接受网络反序列化，且不能用于首次登记或跨主人存储替换。
pub struct PreparedTotpFactor {
    pub(super) context: Arc<totp::Context>,
    store_id: String,
    source_enrollment: i64,
    source_digest: String,
}

impl PreparedTotpFactor {
    /// 固定原登记及新种子。拒绝使用同一引用或在新引用下复用当前秘密。
    pub fn prepare(sessions: Arc<SessionManager>, seed: TotpSeed) -> Result<Self, SessionError> {
        let context = Arc::new(totp::Context {
            sessions: Arc::clone(&sessions),
            seed,
        });
        sessions
            .database
            .transaction(|db| Self::from_context(context, db))
    }

    pub(in crate::session) fn from_context(
        context: Arc<totp::Context>,
        db: &Connection,
    ) -> Result<Self, SessionError> {
        if schema::version(db)? != 4 {
            return Err(SessionError::MigrationRequired);
        }
        let factor = totp::store::factor(db)?.ok_or(SessionError::FactorUnavailable)?;
        let proof = factor.enrollment.ok_or(SessionError::FactorUnavailable)?;
        if factor.seed_ref == context.seed.reference()
            || context
                .seed
                .matches_binding(&factor.seed_ref, &factor.seed_digest)
        {
            return Err(SessionError::Configuration);
        }
        Ok(Self {
            context,
            store_id: schema::store_id(db)?,
            source_enrollment: proof.operation.sequence,
            source_digest: factor.seed_digest,
        })
    }

    pub(in crate::session) fn check(&self, db: &Connection) -> Result<(), SessionError> {
        let factor = totp::store::factor(db)?.ok_or(SessionError::FactorUnavailable)?;
        if self.store_id != schema::store_id(db)?
            || self.source_digest != factor.seed_digest
            || factor
                .enrollment
                .is_none_or(|p| p.operation.sequence != self.source_enrollment)
        {
            return Err(SessionError::Configuration);
        }
        Ok(())
    }
}
