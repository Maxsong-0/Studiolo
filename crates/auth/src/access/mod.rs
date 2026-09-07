//! Access 身份 Module：仅通过固定 team 的 HTTPS JWKS 验证单个人类身份。

mod config;
mod identity;
mod keys;

#[cfg(any(test, feature = "test-support"))]
mod fixture;
#[cfg(feature = "test-support")]
pub use fixture::fixture_verifier;

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
use serde::Deserialize;

pub use config::AccessConfig;
pub use identity::AccessIdentity;
use keys::{HttpsSource, KeyCache, valid_kid};

/// Access 配置、取钥或身份验证失败；不向客户端暴露 JWT 或底层网络错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    /// 配置必须完整且精确，不能由请求提供。
    #[error("Access 配置无效，必须完整指定公共 HTTPS origin、team、audience 和单邮箱")]
    Configuration,
    /// 未取得仍有效的可信密钥；失败关闭。
    #[error("Access 身份验证暂不可用")]
    Unavailable,
    /// 签名或身份声明不符合当前应用策略。
    #[error("Access 身份验证失败")]
    Rejected,
}

/// 可克隆的验证器；共享有界 JWKS 缓存，密钥过期时不能回退到旧密钥。
#[derive(Clone)]
pub struct AccessVerifier {
    config: AccessConfig,
    keys: Arc<KeyCache>,
}

impl AccessVerifier {
    /// 启动时通过 HTTPS 加载密钥，失败则不创建公网入口。
    ///
    /// JWKS 最多 64 KiB / 8 把 RSA 密钥；超时 5 秒，不重定向或使用环境代理。
    /// 缓存 5 分钟，未知 kid 最多每 30 秒刷新一次；Cookie/转发身份不参与验证。
    pub async fn connect(config: AccessConfig) -> Result<Self, AccessError> {
        let source = Arc::new(HttpsSource::new(&config.issuer)?);
        let keys = Arc::new(KeyCache::connect(source).await?);
        Ok(Self { config, keys })
    }

    /// 返回可信配置指定的公共 origin；不采用请求中的转发头。
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.config.public_origin
    }

    /// 验证本次请求的完整 JWT；调用者不得缓存身份来代替后续请求的验证。
    ///
    /// 返回值只证明本次验证已通过；会话仍须再次检查到期与持久身份版本。
    pub async fn verify(&self, token: &str) -> Result<AccessIdentity, AccessError> {
        if token.is_empty()
            || token.len() > 16 * 1024
            || token.split('.').count() != 3
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            return Err(AccessError::Rejected);
        }
        let header = decode_header(token).map_err(|_| AccessError::Rejected)?;
        if header.alg != Algorithm::RS256
            || header.crit.is_some()
            || header.enc.is_some()
            || header.zip.is_some()
            || header.jku.is_some()
            || header.jwk.is_some()
            || header.x5u.is_some()
        {
            return Err(AccessError::Rejected);
        }
        let kid = header
            .kid
            .filter(|kid| valid_kid(kid))
            .ok_or(AccessError::Rejected)?;
        let key = self.keys.key(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);
        validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AccessError::Rejected)?
            .claims;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AccessError::Unavailable)?
            .as_secs();
        if claims.exp <= now
            || claims.nbf > now
            || claims.nbf >= claims.exp
            || claims.iss != self.config.issuer
            || !claims.aud.contains(&self.config.audience)
            || claims.email != self.config.email
            || claims.token_type != "app"
            || claims.sub.trim().is_empty()
            || claims.sub.len() > 256
            || claims.sub.chars().any(char::is_control)
        {
            return Err(AccessError::Rejected);
        }
        Ok(AccessIdentity::verified(
            claims.email,
            claims.sub,
            claims.iss,
            self.config.audience.clone(),
            claims.exp,
        ))
    }
}

#[derive(Clone, Deserialize)]
struct Claims {
    exp: u64,
    nbf: u64,
    iss: String,
    aud: Vec<String>,
    sub: String,
    email: String,
    #[serde(rename = "type")]
    token_type: String,
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

/// 私有单元测试使用的固定虚构身份；不会进入生产或 test-support feature。
#[cfg(test)]
pub(crate) fn test_identity() -> AccessIdentity {
    test_identity_with("fictional-person", 4_102_444_800)
}

/// 仅私有会话契约可调整 subject 与到期；不改变固定虚构应用和邮箱。
#[cfg(test)]
pub(crate) fn test_identity_with(subject: &str, expires_at: u64) -> AccessIdentity {
    AccessIdentity::verified(
        "maxsong@example.com".into(),
        subject.into(),
        "https://studiolo.cloudflareaccess.com".into(),
        "example-audience".into(),
        expires_at,
    )
}

/// 私有闭环测试通过真实 RS256 签名与完整验证取得身份，不绕过验证器构造。
#[cfg(test)]
pub(crate) async fn test_signed_identity() -> AccessIdentity {
    let jwt = test_support::signed(&test_support::claims());
    test_support::verifier().await.verify(&jwt).await.unwrap()
}
