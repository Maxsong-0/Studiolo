//! 可信预配置因子与会话绑定近期认证；无远端登记、恢复或秘密文件读取入口。

pub(in crate::session) mod crypto;
mod enrollment_state;
mod flow;
pub(in crate::session) mod model;
mod pipeline;
mod runtime;
pub(super) mod schema;
mod secrets;
pub(in crate::session) mod store;
mod types;

pub use enrollment_state::TotpEnrollmentState;
pub use secrets::{TotpCode, TotpSeed};
pub use types::{RecentAuthentication, SensitiveAction, TotpRequest};

use super::{SessionError, SessionManager};
use std::sync::Arc;

pub(in crate::session) struct Context {
    pub(in crate::session) sessions: Arc<SessionManager>,
    pub(in crate::session) seed: TotpSeed,
}

/// 普通请求只持此句柄；所有权限与期限在私有 Auth 事务内重新检查。
pub struct Totp {
    pub(in crate::session) context: Arc<Context>,
}

/// 仅交给可信本地装配的首次登记句柄，不得传给 HTTP 路由或反序列化请求。
///
/// 此类型区分进程内职责，不声称提供 OS 隔离；真实 TTY/secret mount 装配尚未交付。
pub struct TotpEnrollment {
    context: Arc<Context>,
}

impl Totp {
    /// 可信装配注入独立密钥平面提供的种子；只接受已显式升级的 v3/v4 AuthDB。
    ///
    /// 不生成秘密、不读任意环境或文件、不登记因子。未来 HTTP 只可获得返回的第一项。
    /// 同一引用背后的种子改变也会拒绝，不提供隐式轮换或恢复。
    pub fn attach(
        sessions: Arc<SessionManager>,
        seed: TotpSeed,
    ) -> Result<(Self, TotpEnrollment), SessionError> {
        sessions.database.transaction(|db| {
            if !matches!(super::schema::version(db)?, 3 | 4) {
                return Err(SessionError::MigrationRequired);
            }
            if let Some(factor) = store::factor(db)?
                && (factor.seed_ref != seed.reference() || factor.seed_digest != seed.fingerprint())
            {
                return Err(SessionError::Configuration);
            }
            Ok(())
        })?;
        let context = Arc::new(Context { sessions, seed });
        Ok((
            Self {
                context: context.clone(),
            },
            TotpEnrollment { context },
        ))
    }
}

/// 会话回收必须保留尚被因子操作引用的对象；只有已确认失效操作可先由补记释放。
pub(super) fn blocks_retention(
    db: &rusqlite::Connection,
    session_id: &str,
) -> Result<bool, SessionError> {
    if super::schema::version(db)? < 3 {
        return Ok(false);
    }
    Ok(store::pending(db)?.is_some_and(|p| p.operation.session_id.as_deref() == Some(session_id)))
}
