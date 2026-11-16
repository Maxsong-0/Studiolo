//! 固定外部端点、有限时限与有限回执；不向调用方传递 reqwest 原始错误。

use super::{AlertMessage, Pushover, Settings, TransportError};
use crate::alert::AlertSeverity;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};
use std::{io::Read, time::Duration};

const PUSHOVER_URL: &str = "https://api.pushover.net/1/messages.json";
const BARK_URL: &str = "https://api.day.app/push";
const MAX_RESPONSE_BYTES: u64 = 4096;

pub(super) struct Options {
    pushover: String,
    bark: String,
    timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pushover: PUSHOVER_URL.into(),
            bark: BARK_URL.into(),
            timeout: Duration::from_secs(8),
        }
    }
}

impl Options {
    #[cfg(test)]
    pub(super) fn for_test(pushover: &str, bark: &str, timeout: Duration) -> Self {
        for endpoint in [pushover, bark] {
            let url = reqwest::Url::parse(endpoint).expect("假服务地址必须合法");
            assert_eq!(url.scheme(), "http");
            assert_eq!(url.host_str(), Some("127.0.0.1"));
        }
        Self {
            pushover: pushover.into(),
            bark: bark.into(),
            timeout,
        }
    }
}

pub(super) fn deliver(
    options: &Options,
    settings: &Settings,
    message: &AlertMessage,
) -> Result<(), TransportError> {
    let client = Client::builder()
        .timeout(options.timeout)
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| TransportError::Unavailable)?;
    let body = format!("Needs attention: code {}", message.code());
    let critical = message.severity() == AlertSeverity::Critical;
    let main = settings.pushover.as_ref().map_or_else(
        || Err(TransportError::NotConfigured),
        |credentials| {
            pushover(
                &client,
                options,
                credentials,
                settings.link.as_deref(),
                &body,
                critical,
            )
        },
    );
    let main_error = match main {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let backup = settings.bark.as_ref().map_or_else(
        || Err(TransportError::NotConfigured),
        |device| {
            bark(
                &client,
                options,
                device,
                settings.link.as_deref(),
                &body,
                critical,
            )
        },
    );
    backup.map_err(|backup_error| combined(main_error, backup_error))
}

fn pushover(
    client: &Client,
    options: &Options,
    credentials: &Pushover,
    link: Option<&str>,
    body: &str,
    critical: bool,
) -> Result<(), TransportError> {
    let mut fields = vec![
        ("token", credentials.token.as_str()),
        ("user", credentials.user.as_str()),
        ("message", body),
        ("priority", if critical { "2" } else { "0" }),
    ];
    if critical {
        fields.extend([("retry", "60"), ("expire", "3000")]);
    }
    if let Some(link) = link {
        fields.push(("url", link));
    }
    // 请求自身也设置时限，确保异步内核的总时限覆盖响应体读取，
    // 而不只是 blocking 外壳每次 read 的等待预算。
    let response = client
        .post(&options.pushover)
        .timeout(options.timeout)
        .form(&fields)
        .send()
        .map_err(network_error)?;
    let value = response_json(response)?;
    if value.get("status").and_then(Value::as_u64) != Some(1)
        || (critical
            && !value
                .get("receipt")
                .and_then(Value::as_str)
                .is_some_and(|receipt| {
                    !receipt.is_empty()
                        && receipt.len() <= 256
                        && !receipt.chars().any(char::is_control)
                }))
    {
        return Err(TransportError::Rejected);
    }
    Ok(())
}

fn bark(
    client: &Client,
    options: &Options,
    device: &str,
    link: Option<&str>,
    body: &str,
    critical: bool,
) -> Result<(), TransportError> {
    let mut payload = json!({
        "device_key": device,
        "body": body,
        "level": if critical { "critical" } else { "passive" },
        "isArchive": 0,
    });
    if critical {
        payload["volume"] = json!(10);
        payload["call"] = json!(1);
    }
    if let Some(link) = link {
        payload["url"] = json!(link);
    }
    let response = client
        .post(&options.bark)
        .timeout(options.timeout)
        .json(&payload)
        .send()
        .map_err(network_error)?;
    if response_json(response)?.get("code").and_then(Value::as_u64) != Some(200) {
        return Err(TransportError::Rejected);
    }
    Ok(())
}

fn response_json(response: Response) -> Result<Value, TransportError> {
    let status = response.status();
    if !status.is_success() {
        return Err(if status.as_u16() == 429 || status.is_server_error() {
            TransportError::Unavailable
        } else {
            TransportError::Rejected
        });
    }
    let mut bytes = Vec::with_capacity((MAX_RESPONSE_BYTES + 1) as usize);
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            let timed_out = error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout);
            if timed_out {
                TransportError::Timeout
            } else {
                TransportError::Unavailable
            }
        })?;
    if bytes.len() > MAX_RESPONSE_BYTES as usize {
        return Err(TransportError::Rejected);
    }
    serde_json::from_slice(&bytes).map_err(|_| TransportError::Rejected)
}

fn network_error(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else {
        TransportError::Unavailable
    }
}

fn combined(main: TransportError, backup: TransportError) -> TransportError {
    use TransportError::{NotConfigured, Rejected, Timeout, Unavailable};
    match (main, backup) {
        (NotConfigured, NotConfigured) => NotConfigured,
        (Timeout, _) | (_, Timeout) => Timeout,
        (Unavailable, _) | (_, Unavailable) => Unavailable,
        _ => Rejected,
    }
}
