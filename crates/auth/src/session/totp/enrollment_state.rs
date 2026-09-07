//! 可信本地登记的非秘密显示投影；不保留或授予任何登记资格。

use super::super::{SessionError, recovery};
use super::{TotpEnrollment, flow, store};

/// 首次登记句柄此刻观察到的状态；不是授权凭据，也不代表本调用已完成登记。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TotpEnrollmentState {
    /// 尚未登记且没有受保护决定；调用 `enroll` 时仍须重新验证所有条件。
    Ready,
    /// 仍有受保护决定，包括已到期但尚未通过显式补记安全清理的决定。
    Pending,
    /// 因子已经登记；再次调用首次登记入口必须拒绝，不能作为本次成功。
    Enrolled,
}

impl TotpEnrollment {
    /// 经完整本地存储/审计回执/种子绑定及时钟检查，返回非秘密登记状态。
    ///
    /// 不调用外部审计、不消费验证码、不补记或清理 pending，更不会激活。
    /// 仍沿会话运行检查提交时钟及到期状态；已登记优先于其他操作的 pending 显示。
    pub fn state(&self) -> Result<TotpEnrollmentState, SessionError> {
        self.context.sessions.run(|db, _| {
            match self.context.factor(db, true) {
                Err(SessionError::AlreadyEnrolled) => {
                    return Ok(Ok(TotpEnrollmentState::Enrolled));
                }
                Err(error) => return flow::expected(db, error),
                Ok(_) => {}
            }
            let state = if store::pending(db)?.is_some() || recovery::has_pending(db)? {
                TotpEnrollmentState::Pending
            } else {
                TotpEnrollmentState::Ready
            };
            Ok(Ok(state))
        })
    }
}
