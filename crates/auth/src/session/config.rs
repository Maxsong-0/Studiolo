//! 只由可信程序显式装配的主人绑定与 TTL，不接受远端抢注。

use super::SessionError;
use crate::AccessIdentity;
use serde::Serialize;

/// 固定主人身份，全部字段必须与本次真实 Access 验证结果精确一致。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnerBinding {
    issuer: String,
    audience: String,
    subject: String,
    email: String,
}

impl OwnerBinding {
    /// 构造非秘密主人配置；不是身份验证或本地来源授权。
    pub fn new(
        issuer: String,
        audience: String,
        subject: String,
        email: String,
    ) -> Result<Self, SessionError> {
        let fields = [
            (&issuer, 512),
            (&audience, 256),
            (&subject, 256),
            (&email, 254),
        ];
        if fields.iter().any(|(value, maximum)| {
            value.trim().is_empty() || value.len() > *maximum || value.chars().any(char::is_control)
        }) || !issuer.starts_with("https://")
        {
            return Err(SessionError::Configuration);
        }
        Ok(Self {
            issuer,
            audience,
            subject,
            email,
        })
    }

    pub(super) fn matches(&self, identity: &AccessIdentity) -> bool {
        identity.issuer() == self.issuer
            && identity.audience() == self.audience
            && identity.subject() == self.subject
            && identity.email() == self.email
    }
}

/// 显式秒数 TTL；0、闲置期限大于绝对期限、绝对期限超过七天均拒绝。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SessionPolicy {
    pub(super) idle_seconds: u64,
    pub(super) absolute_seconds: u64,
}

impl SessionPolicy {
    /// 构造明确的闲置和绝对期限，不提供隐式部署默认值。
    pub fn new(idle_seconds: u64, absolute_seconds: u64) -> Result<Self, SessionError> {
        if idle_seconds == 0
            || idle_seconds > absolute_seconds
            || absolute_seconds > 7 * 24 * 60 * 60
        {
            return Err(SessionError::Configuration);
        }
        Ok(Self {
            idle_seconds,
            absolute_seconds,
        })
    }
}

/// 重开必须与磁盘主人和 TTL 完全一致；更改不是隐式迁移或全部重新授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionConfig {
    pub(super) owner: OwnerBinding,
    pub(super) policy: SessionPolicy,
}

impl SessionConfig {
    /// 将经可信程序批准的主人与期限绑定为持久配置。
    #[must_use]
    pub fn new(owner: OwnerBinding, policy: SessionPolicy) -> Self {
        Self { owner, policy }
    }

    pub(super) fn encoded(&self) -> Result<String, SessionError> {
        serde_json::to_string(self).map_err(|_| SessionError::Configuration)
    }
}
