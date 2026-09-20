//! 公共入口只接收凭据和封闭动作，不暴露 pending、审计回执或激活能力。

use super::super::{SessionError, denials::Denial, recent, state};
use super::{RecentAuthentication, SensitiveAction, Totp, TotpCode, TotpEnrollment, TotpRequest};

impl TotpEnrollment {
    /// 以可信装配的主人及预配置种子完成首次登记；不授予任何应用会话 L3。
    ///
    /// 真实 TTY/秘密读取和恢复码流程仍为后续装配；没有网络登记或隐式覆盖路径。
    pub fn enroll(&self, code: TotpCode) -> Result<(), SessionError> {
        self.context.execute(None, &code).map(|_| ())
    }
}

impl Totp {
    /// 在持久预扣预算后消费一次正确验证码，审计双确认并重新验权后激活当前会话。
    ///
    /// 失败可能已消费验证码；补记不能恢复本次提权。审计等待不延长显示的期限。
    pub fn step_up(
        &self,
        request: &TotpRequest<'_>,
        code: TotpCode,
    ) -> Result<RecentAuthentication, SessionError> {
        self.context.execute(Some(request), &code)
    }

    /// 在具体动作的 intent 之后、执行或最终切换之前即时检查；不能缓存此结果。
    ///
    /// 此检查不代替目标 Module 的请求参数验证/审计，也不声称鉴权与外部副作用原子。
    pub fn require_l3(
        &self,
        request: &TotpRequest<'_>,
        action: SensitiveAction,
    ) -> Result<(), SessionError> {
        // 显式封闭匹配：未来新增动作必须先经过权威矩阵裁决。
        match action {
            SensitiveAction::ActivatePolicy
            | SensitiveAction::ApproveCandidate
            | SensitiveAction::ChangeVault
            | SensitiveAction::ChangeBackup
            | SensitiveAction::ChangeSync
            | SensitiveAction::RestoreVault
            | SensitiveAction::ReleaseFreeze
            | SensitiveAction::TestCredential
            | SensitiveAction::ManageRecoveryCodes => {}
        }
        let context = &self.context;
        context.sessions.run(|db, observed| {
            let row = match context.sessions.authorized(
                db,
                observed,
                request.identity,
                request.token,
                Some(request.csrf),
            )? {
                Ok(row) => row,
                Err(error) => return Ok(Err(error)),
            };
            let factor = match context.factor(db, false) {
                Ok(factor) => factor,
                Err(error) => return super::flow::expected(db, error),
            };
            if factor.is_none() {
                return state::deny(db, Denial::Issue, SessionError::FactorUnavailable);
            }
            if recent::authentication(db, observed, row.info.id.as_str())?.is_none() {
                return state::deny(db, Denial::Issue, SessionError::ReauthenticationRequired);
            }
            Ok(Ok(()))
        })
    }

    /// 补记两条原因子事件并重确认最多四条旧证明；仅释放双确认的失效 pending，绝不激活。
    ///
    /// 拒绝聚合继续由 `SessionManager::flush_audit` 交付；本方法不承诺清空所有审计工作。
    pub fn flush_audit(&self) -> Result<usize, SessionError> {
        let delivered = self.context.deliver(None)?;
        self.context.sessions.run(|db, observed| {
            super::pipeline::release_invalid(db, observed)?;
            Ok(Ok(()))
        })?;
        Ok(delivered)
    }
}
