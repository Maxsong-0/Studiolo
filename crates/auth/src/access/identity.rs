//! 只能由 Access Module 生成的签名身份；不代表持续有效的应用会话。

/// 已通过签名、完整声明和固定配置检查的身份。
///
/// 本类型只证明曾通过验证。每个 HTTP 请求必须重新验证 JWT，使用会话时还须
/// 检查当前时间、持久身份版本和吊销状态；克隆不能代替这些检查。
///
/// 外部不能以结构体字面量伪造验证结果：
/// ```compile_fail
/// use studiolo_auth::AccessIdentity;
/// let identity = AccessIdentity {
///     email: "owner@example.com".into(), subject: "owner".into(),
///     issuer: "https://example.cloudflareaccess.com".into(),
///     audience: "application".into(), expires_at: u64::MAX,
/// };
/// ```
/// 不提供绕过验证的构造器：
/// ```compile_fail
/// use studiolo_auth::AccessIdentity;
/// let identity = AccessIdentity::new("owner@example.com", "owner");
/// ```
/// 请求正文也不能反序列化为可信身份：
/// ```compile_fail
/// use studiolo_auth::AccessIdentity;
/// let identity = serde_json::from_str::<AccessIdentity>("{}");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessIdentity {
    email: String,
    subject: String,
    issuer: String,
    audience: String,
    expires_at: u64,
}

impl AccessIdentity {
    // 仅父模块的验证路径及 cfg(test) 夹具可调用；会话模块不能自行构造。
    pub(super) fn verified(
        email: String,
        subject: String,
        issuer: String,
        audience: String,
        expires_at: u64,
    ) -> Self {
        Self {
            email,
            subject,
            issuer,
            audience,
            expires_at,
        }
    }

    /// 与可信配置精确相等的签名邮箱。
    #[must_use]
    pub fn email(&self) -> &str {
        &self.email
    }

    /// 非空且有界的签名 subject。
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// 与固定 team 配置精确相等的签名 issuer。
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// 已验证包含在签名 aud 中的配置 audience，不采用数组中的任意首项。
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// 签名 exp，以 UNIX 秒表示；使用时仍须重新检查。
    #[must_use]
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}
