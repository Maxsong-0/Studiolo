//! 单次事务的非授权显示投影；不消费、补记、清理或发布因子决定。

use super::{Active, FactorRuntime};
use crate::session::{denials::Denial, recent, recovery, state, totp};
use crate::{AccessIdentity, CsrfToken, RecentAuthentication, SessionError, SessionToken};

/// 当前会话与运行因子的一次观察；既不是授权票据，也不是先前请求的完成回执。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactorState {
    /// Auth本次观察的服务端时刻；显示倒计时不能延长实际认证期限。
    pub observed_at_ms: i64,
    /// 已激活且此刻未到期的当前会话近期认证；具体动作仍须重新验权。
    pub recent_authentication: Option<RecentAuthentication>,
    /// 原活动槽仍匹配预配置恢复候选；不代表已有恢复码或当前动作获准。
    pub recovery_candidate_available: bool,
    /// 仍存在受保护决定，包括过期但尚未显式或惰性清理的决定。
    /// false不证明其他调用尚未执行，true也不能永久禁止用户发起独立新动作。
    pub pending: bool,
}

impl FactorRuntime {
    /// 在同一Auth事务内重验身份、会话与CSRF，选择权威槽并返回非秘密显示状态。
    ///
    /// 保留原时钟地板、会话到期及last_seen语义；不调用外部审计、不取随机数，
    /// 不扣验证预算、不消费验证码、不补记或清理pending，更不会激活权限。
    /// 未知绑定及存储故障照常失败，不能解释成因子能力不存在。
    pub fn state(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
    ) -> Result<FactorState, SessionError> {
        self.sessions.run(|db, observed| {
            let selected = match self.select_in(db, observed, identity, token, csrf)? {
                Ok(selected) => selected,
                Err(error) => return Ok(Err(error)),
            };
            let mut recovery_candidate_available = false;
            if selected.active == Active::Current
                && let Some(prepared) = &self.prepared
            {
                if let Err(error) = prepared.check(db) {
                    return match error {
                        SessionError::Configuration | SessionError::FactorUnavailable => {
                            state::deny(db, Denial::Issue, error)
                        }
                        _ => Err(error),
                    };
                }
                recovery_candidate_available = true;
            }
            Ok(Ok(FactorState {
                observed_at_ms: observed.now,
                recent_authentication: recent::authentication(
                    db,
                    observed,
                    selected.session.id.as_str(),
                )?,
                recovery_candidate_available,
                pending: totp::store::pending(db)?.is_some() || recovery::has_pending(db)?,
            }))
        })
    }
}
