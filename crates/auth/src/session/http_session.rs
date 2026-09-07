//! 网页会话的闭合 Auth 策略；不解析 Cookie，不接受因子秘密或缓存的登记状态。

use super::{
    CsrfToken, IssuedSession, SessionError, SessionInfo, SessionManager, SessionToken,
    denials::Denial, schema, state, totp,
};
use crate::AccessIdentity;
use rusqlite::Connection;

#[derive(Clone, Copy)]
pub(super) enum IssuePolicy {
    Ordinary,
    Enrolled,
}

impl SessionManager {
    /// 为网页签发会话；当前主人和已确认登记必须在每个签发事务中继续有效。
    ///
    /// 初检同时观察会话到期，再执行一轮既有有界回收（最多 16 个终态）；
    /// 维护或审计失败关闭新授权，不自动重试，不承诺后台维护。无需注入因子种子。
    pub fn issue_enrolled(&self, identity: &AccessIdentity) -> Result<IssuedSession, SessionError> {
        self.issue_run(identity, IssuePolicy::Enrolled, |_, _| Ok(Ok(())))?;
        if let Err(error) = self.maintain_retention() {
            return self.issue_run(identity, IssuePolicy::Enrolled, |db, _| {
                state::deny(db, Denial::Issue, error)
            });
        }
        self.issue_with_policy(identity, IssuePolicy::Enrolled)
    }

    /// 在同一事务内验证真实 Access、会话及其 CSRF 配对，返回非秘密元信息。
    ///
    /// 这仍是普通读取检查；结果不能缓存为授权，也不授予内容写入或近期认证权限。
    pub fn check_with_csrf(
        &self,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
    ) -> Result<SessionInfo, SessionError> {
        self.run(|db, observed| {
            Ok(self
                .authorized(db, observed, identity, token, Some(csrf))?
                .map(|row| row.info))
        })
    }

    pub(super) fn issue_run<T>(
        &self,
        identity: &AccessIdentity,
        policy: IssuePolicy,
        operation: impl FnOnce(
            &Connection,
            &state::Observed,
        ) -> Result<Result<T, SessionError>, SessionError>,
    ) -> Result<T, SessionError> {
        self.run(|db, observed| {
            if matches!(policy, IssuePolicy::Enrolled) {
                if let Err(error) = self.identity(db, identity, observed.now)? {
                    return Ok(Err(error));
                }
                // run 已完整验证本地登记及原回执；pending 或消费锚点本身不能作为登记。
                let enrolled = schema::version(db)? >= 3
                    && totp::store::factor(db)?.is_some_and(|factor| factor.enrollment.is_some());
                if !enrolled {
                    return state::deny(db, Denial::Issue, SessionError::FactorUnavailable);
                }
            }
            operation(db, observed)
        })
    }
}
