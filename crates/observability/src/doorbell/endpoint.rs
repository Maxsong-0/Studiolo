//! 门铃的收件配置与真实发送。**门铃只在这个文件里做网络 IO。**
//!
//! 收件地址和签名密钥只从环境读（systemd 的 600 环境文件注入），本文件只出现环境变量的
//! **名字**。错误与 `Debug` 不含地址、密钥、正文或远端回执。

use super::{Secret, signature};
use reqwest::blocking::Client;
use std::{fmt, time::Duration};

/// 收件地址（HTTPS）。
pub const URL_VARIABLE: &str = "STUDIOLO_DOORBELL_URL";
/// 签名密钥（`whsec_` 开头）。
pub const SECRET_VARIABLE: &str = "STUDIOLO_DOORBELL_SECRET";
/// 可选：放进事件里的、要登录才能打开的 HTTPS 链接。
pub const LINK_VARIABLE: &str = "STUDIOLO_DOORBELL_LINK";

const TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const USER_AGENT: &str = "studiolo-doorbell/1";
const MAX_MESSAGE_ID_BYTES: usize = 64;

/// 一次投递失败的固定分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PostError {
    /// 没配收件地址和密钥。这是明确的「没配」，不是成功。
    #[error("门铃收件端未配置")]
    NotConfigured,
    /// 配置不合法（只给了一半、地址不是合规的 HTTPS、密钥不合规范）。
    #[error("门铃收件配置不合法")]
    Malformed,
    /// 超时；对方可能已经收到。
    #[error("门铃投递超时")]
    Timeout,
    /// 连不上、被限流或对方服务端出错；稍后再试。
    #[error("门铃收件端暂不可用")]
    Unavailable,
    /// 对方明确不收（含 3xx：不跟重定向）。
    #[error("门铃收件端拒收")]
    Rejected,
    /// 对方回 410：这个地址不再接收，停用到换地址为止。
    #[error("门铃收件端已停用")]
    Gone,
}

impl PostError {
    /// 写进审计的固定机器码。
    #[must_use]
    pub fn classification(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::Malformed => "malformed",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::Rejected => "rejected",
            Self::Gone => "gone",
        }
    }
}

/// 门铃的发送接缝。真实实现是 [`HttpEndpoint`]；测试用假实现。
pub trait DoorbellTransport {
    /// 当前收件地址的指纹（16 位十六进制）；换了地址指纹就变。不含地址本身。
    fn endpoint_tag(&self) -> Result<String, PostError>;
    /// 放进事件正文的可选链接。
    fn link(&self) -> Option<&str>;
    /// 签名并投递一条事件；`Ok` 只表示对方的服务收下了。
    fn post(&self, message_id: &str, timestamp_secs: i64, body: &[u8]) -> Result<(), PostError>;
}

struct Settings {
    url: reqwest::Url,
    secret: Secret,
    link: Option<String>,
    tag: String,
}

/// 按 Standard Webhooks 1.0.0 签名后 POST 到一个固定的 HTTPS 地址。
///
/// 不跟重定向、不走代理、有总时限。内部用阻塞客户端：只能从同步上下文调用。
pub struct HttpEndpoint {
    settings: Result<Settings, PostError>,
    timeout: Duration,
}

impl HttpEndpoint {
    /// 从环境读收件地址、密钥与可选链接。此时不创建 HTTP 客户端，也不访问网络。
    #[must_use]
    pub fn from_environment() -> Self {
        Self::from_lookup(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(PostError::Malformed),
        })
    }

    /// 地址和密钥都给了且合法。
    #[must_use]
    pub fn configured(&self) -> bool {
        self.settings.is_ok()
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Result<Option<String>, PostError>) -> Self {
        let settings = (|| {
            let url = lookup(URL_VARIABLE)?;
            let secret = lookup(SECRET_VARIABLE)?;
            let link = lookup(LINK_VARIABLE)?;
            let (url, secret) = match (url, secret) {
                (None, None) => return Err(PostError::NotConfigured),
                (Some(url), Some(secret)) => (url, secret),
                // 只给了一半：不猜另一半，也不当成「没配」悄悄跳过。
                _ => return Err(PostError::Malformed),
            };
            let url = crate::https_url::checked(&url, true).ok_or(PostError::Malformed)?;
            Self::settings(url, &secret, link)
        })();
        Self {
            settings,
            timeout: TIMEOUT,
        }
    }

    fn settings(
        url: reqwest::Url,
        secret: &str,
        link: Option<String>,
    ) -> Result<Settings, PostError> {
        let secret = Secret::parse(secret).map_err(|_| PostError::Malformed)?;
        let link = link
            .map(|link| {
                crate::https_url::checked(&link, false)
                    .map(String::from)
                    .ok_or(PostError::Malformed)
            })
            .transpose()?;
        let tag = endpoint_tag(url.as_str());
        Ok(Settings {
            url,
            secret,
            link,
            tag,
        })
    }

    /// 只给测试：指向本机回环上的假服务（明文 HTTP），可缩短时限。
    #[cfg(test)]
    fn for_test(url: &str, secret: &str, link: Option<&str>, timeout: Duration) -> Self {
        let url = reqwest::Url::parse(url).expect("假服务地址必须合法");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        Self {
            settings: Self::settings(url, secret, link.map(str::to_owned)),
            timeout,
        }
    }
}

impl fmt::Debug for HttpEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpEndpoint")
            .field("configured", &self.configured())
            .finish()
    }
}

impl DoorbellTransport for HttpEndpoint {
    fn endpoint_tag(&self) -> Result<String, PostError> {
        self.settings
            .as_ref()
            .map(|settings| settings.tag.clone())
            .map_err(|error| *error)
    }

    fn link(&self) -> Option<&str> {
        self.settings
            .as_ref()
            .ok()
            .and_then(|settings| settings.link.as_deref())
    }

    fn post(&self, message_id: &str, timestamp_secs: i64, body: &[u8]) -> Result<(), PostError> {
        let settings = self.settings.as_ref().map_err(|error| *error)?;
        if !valid_message_id(message_id) {
            return Err(PostError::Malformed);
        }
        let signature = signature(&settings.secret, message_id, timestamp_secs, body)
            .map_err(|_| PostError::Malformed)?;
        let client = Client::builder()
            .timeout(self.timeout)
            .connect_timeout(CONNECT_TIMEOUT.min(self.timeout))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| PostError::Unavailable)?;
        // 请求自身也设时限，让总时限覆盖到响应头读完，而不只是每次读写的等待。
        let response = client
            .post(settings.url.clone())
            .timeout(self.timeout)
            .header("content-type", "application/json")
            .header("webhook-id", message_id)
            .header("webhook-timestamp", timestamp_secs.to_string())
            .header("webhook-signature", signature)
            .body(body.to_vec())
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    PostError::Timeout
                } else {
                    PostError::Unavailable
                }
            })?;
        // 只看状态码，不读对方的回执正文。
        match response.status().as_u16() {
            200..=299 => Ok(()),
            410 => Err(PostError::Gone),
            408 | 429 | 500..=599 => Err(PostError::Unavailable),
            _ => Err(PostError::Rejected),
        }
    }
}

fn valid_message_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_MESSAGE_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// 收件地址的指纹：用来发现「换了地址」，不用来还原地址。
fn endpoint_tag(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"studiolo.doorbell.endpoint.v1\0");
    digest.update(url.as_bytes());
    let hex = b"0123456789abcdef";
    digest
        .finalize()
        .iter()
        .take(8)
        .flat_map(|byte| {
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/support/doorbell_endpoint.rs"]
mod tests;
