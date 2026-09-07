//! 固定受信任配置；不从请求推导身份与取钥地址。

use reqwest::Url;

use super::AccessError;

/// 显式 Access 配置；所有字段是非秘密标识，不能从转发头或 JWT 中推导。
#[derive(Clone, Debug)]
pub struct AccessConfig {
    pub(super) public_origin: String,
    pub(super) issuer: String,
    pub(super) audience: String,
    pub(super) email: String,
}

impl AccessConfig {
    /// 只接受规范 HTTPS origin、单标签 Cloudflare team、单 audience 与单邮箱。
    ///
    /// Origin 不带尾斜杠、路径、用户信息、查询或片段；邮箱区分大小写精确匹配。
    pub fn new(
        public_origin: String,
        team_issuer: String,
        audience: String,
        email: String,
    ) -> Result<Self, AccessError> {
        let url = Url::parse(&public_origin).map_err(|_| AccessError::Configuration)?;
        let public_ok = public_origin.len() <= 300
            && url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.origin().ascii_serialization() == public_origin
            && url.domain().is_some_and(|host| host.contains('.'));
        let label = team_issuer
            .strip_prefix("https://")
            .and_then(|host| host.strip_suffix(".cloudflareaccess.com"));
        let team_ok = label.is_some_and(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
        });
        let email_ok = email.len() <= 254
            && email.bytes().all(|b| b.is_ascii_graphic())
            && email.split('@').count() == 2
            && email.split_once('@').is_some_and(|(name, host)| {
                !name.is_empty() && !host.is_empty() && host.contains('.')
            });
        if !public_ok
            || !team_ok
            || !email_ok
            || audience.is_empty()
            || audience.len() > 256
            || !audience.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AccessError::Configuration);
        }
        Ok(Self {
            public_origin,
            issuer: team_issuer,
            audience,
            email,
        })
    }
}
