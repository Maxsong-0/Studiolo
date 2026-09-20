//! 请求携带原始已验证身份，仍由 Auth 每次重验；来源地址不是授权凭据。

use crate::{AccessIdentity, CsrfToken, SessionToken};
use std::net::IpAddr;

/// 固定可信入口组装的请求；HTTP 还必须检查 Origin/Fetch Metadata 和代理来源。
///
/// `source` 必须来自可信代理规则，不可直接使用任意 X-Forwarded-For。
pub struct TotpRequest<'a> {
    /// 本次新验的 Access 身份，不能以旧会话替代 JWT 检查。
    pub identity: &'a AccessIdentity,
    /// 当前应用会话凭据。
    pub token: &'a SessionToken,
    /// 当前会话绑定的 CSRF 凭据。
    pub csrf: &'a CsrfToken,
    /// 可信装配取得的真实客户端地址；类型本身不证明其来源。
    pub source: IpAddr,
}

/// 权威矩阵中封闭的 L3 动作，调用方不能自行指定认证等级。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitiveAction {
    /// 发布策略、回滚策略以及通过策略启用 Adapter。
    ActivatePolicy,
    /// 审阅已保留的执行体候选并确认逐项验收。
    ApproveCandidate,
    /// 改变预批准范围内的 canonical Vault 目标。
    ChangeVault,
    /// 改变备份目标及故障域声明。
    ChangeBackup,
    /// 改变 Sync 目标与白名单。
    ChangeSync,
    /// 整库恢复的执行或最终切换。
    RestoreVault,
    /// 解除全局冻结。
    ReleaseFreeze,
    /// 测试指定凭据连接。
    TestCredential,
    /// 生成或轮换整批恢复码；必须具有当前因子的近期认证。
    ManageRecoveryCodes,
}

/// 已完成当前会话近期认证的显示信息，不是可缓存的授权凭据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecentAuthentication {
    /// 正确验证码消费时刻，审计等待不重置此时间。
    pub verified_at_ms: i64,
    /// 不含边界点的期限；执行具体动作时仍须调用 `require_l3`。
    pub expires_at_ms: i64,
}
