//! 两级推送：主通道确认接受后结束，未确认才尝试备用通道。
//!
//! 配置值只驻内存；错误与 Debug 不含密钥、地址、正文或远端回执。

mod http;

#[cfg(test)]
#[path = "../../tests/support/push.rs"]
mod tests;

use super::{AlertMessage, AlertTransport, DeliveryReceipt, TransportError};
use std::{fmt, sync::Mutex};

const TOKEN: &str = "STUDIOLO_ALERT_PUSHOVER_TOKEN";
const USER: &str = "STUDIOLO_ALERT_PUSHOVER_USER";
const DEVICE: &str = "STUDIOLO_ALERT_BARK_DEVICE_KEY";
const LINK: &str = "STUDIOLO_ALERT_LINK";
const MAX_SECRET_BYTES: usize = 1024;
const MAX_CODE_BYTES: usize = 64;

struct Pushover {
    token: String,
    user: String,
}

struct Settings {
    pushover: Option<Pushover>,
    bark: Option<String>,
    link: Option<String>,
}

/// Pushover 主通道与 Bark 备用通道；只发送固定代号和可选 HTTPS 链接。
///
/// 回执仅证明服务接受，不能证明手机已响或主人已读。远端超时可能已发送，
/// 同键去重仍由 [`super::AlertDispatcher`] 负责，不承诺远端恰好一次。
pub struct PushTransport {
    settings: Result<Settings, TransportError>,
    serial: Mutex<()>,
    http: http::Options,
}

impl PushTransport {
    /// 从环境读取主通道 token/user、备用 device key 与可选链接。
    ///
    /// 任一已给配置不合法即失败关闭；缺一种通道仍可使用另一种。
    /// 此时不创建 HTTP 客户端，也不访问网络。
    #[must_use]
    pub fn from_environment() -> Self {
        Self::from_lookup(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(TransportError::Malformed),
        })
    }

    /// 至少一种通道完整配置且所有已给值合法。
    #[must_use]
    pub fn configured(&self) -> bool {
        self.settings
            .as_ref()
            .is_ok_and(|settings| settings.pushover.is_some() || settings.bark.is_some())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Result<Option<String>, TransportError>) -> Self {
        let settings = (|| {
            let token = secret(lookup(TOKEN)?)?;
            let user = secret(lookup(USER)?)?;
            let bark = secret(lookup(DEVICE)?)?;
            let link = lookup(LINK)?.map(valid_link).transpose()?;
            let pushover = token
                .zip(user)
                .map(|(token, user)| Pushover { token, user });
            Ok(Settings {
                pushover,
                bark,
                link,
            })
        })();
        Self {
            settings,
            serial: Mutex::new(()),
            http: http::Options::default(),
        }
    }
}

impl fmt::Debug for PushTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PushTransport")
            .field("configured", &self.configured())
            .finish()
    }
}

impl AlertTransport for PushTransport {
    fn deliver(&self, message: &AlertMessage) -> Result<DeliveryReceipt, TransportError> {
        let settings = self.settings.as_ref().map_err(|error| *error)?;
        if !valid_code(message.code()) {
            return Err(TransportError::Malformed);
        }
        if settings.pushover.is_none() && settings.bark.is_none() {
            return Err(TransportError::NotConfigured);
        }
        let _serial = self
            .serial
            .lock()
            .map_err(|_| TransportError::Unavailable)?;
        // blocking 客户端自带运行时；创建、使用与销毁全部留在独立线程，
        // 即使同步 trait 从 Tokio 任务调用，也不会在调用方上下文销毁运行时。
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("studiolo-alert-push".into())
                .spawn_scoped(scope, || http::deliver(&self.http, settings, message))
                .map_err(|_| TransportError::Unavailable)?
                .join()
                .map_err(|_| TransportError::Unavailable)?
        })?;
        Ok(DeliveryReceipt::new(message.key().clone()))
    }
}

fn secret(value: Option<String>) -> Result<Option<String>, TransportError> {
    if value.as_ref().is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_SECRET_BYTES
            || value
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
    }) {
        return Err(TransportError::Malformed);
    }
    Ok(value)
}

fn valid_link(value: String) -> Result<String, TransportError> {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
        || value.contains('\\')
    {
        return Err(TransportError::Malformed);
    }
    let url = reqwest::Url::parse(&value).map_err(|_| TransportError::Malformed)?;
    // URL 解析会消去空 userinfo；另查原始 authority，连 https://@host 也拒绝。
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default())
        .ok_or(TransportError::Malformed)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || authority.contains('@')
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(TransportError::Malformed);
    }
    Ok(url.into())
}

fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
