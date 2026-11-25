//! 私有 Unix 转发：固定地址、无代理、无重定向；外壳不获得 canonical 文件能力。
use reqwest::StatusCode;
use std::{path::Path, time::Duration};

/// 请求正文上限；超了就不发。
const REQUEST_LIMIT: usize = 262_144;
/// 响应正文上限；读到超了就放弃这次应答。
const RESPONSE_LIMIT: usize = 1_048_576;
/// 回给 AI 的上游错误文案最多保留的字符数。
const MESSAGE_LIMIT: usize = 512;
/// 4xx 正文没给原因时的文案。
const REJECTED: &str = "黑板请求被权威服务拒绝";
/// 5xx 等正文没给原因时的文案。
const UNAVAILABLE: &str = "黑板权威服务暂不可用，请稍后重试";

/// 黑板调用失败的两种性质，调用方必须分开处理。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardCallError {
    /// 权威服务明确拒绝了这次请求（4xx），或请求本身发不出去（不可编码、超上限）：重试无用。
    Rejected(String),
    /// 这一刻拿不到结论：连不上、超时、5xx、响应读不完、超上限或不是 JSON。稍后可重试。
    Unavailable(String),
}

/// 连接器调用的失败（ADR-046）；只认权威服务给的代码，认不出的一律是这一刻给不出结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorError {
    /// 身份或策略确定不放行。
    Denied,
    /// 没有这条，或不在名单里。
    NotFound,
    /// `seq` 不比上一份大。
    Stale,
    /// 字段不合。
    Invalid,
    /// 连不上、超时、5xx、名额满、旧的权威服务没有这个入口。
    Unavailable,
}

/// 通向既有 canonical 单写者的有界连接。
pub struct BoardClient {
    client: reqwest::Client,
}

impl BoardClient {
    /// 从明确配置的绝对 Unix socket 构造；权限和 peer 由接收端核验。
    pub fn new(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || path.as_os_str().len() > 103 {
            return Err("黑板 Unix 路径无效".into());
        }
        let client = reqwest::Client::builder()
            .unix_socket(path.to_owned())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "黑板连接配置失败".to_owned())?;
        Ok(Self { client })
    }

    /// 仅向固定入口转发结构化请求，限制请求和响应尺寸。
    pub async fn call(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, BoardCallError> {
        let bytes = serde_json::to_vec(request)
            .map_err(|_| BoardCallError::Rejected("黑板请求不可编码".into()))?;
        if bytes.len() > REQUEST_LIMIT {
            return Err(BoardCallError::Rejected("黑板请求超过上限".into()));
        }
        let (status, body) = self
            .post("http://localhost/internal/board", bytes)
            .await
            .map_err(|message| BoardCallError::Unavailable(message.into()))?;
        classify(status, &body)
    }

    /// 连接器的固定私有入口；同一条连接、同一份尺寸上限。
    pub async fn connect(
        &self,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, ConnectorError> {
        let bytes = serde_json::to_vec(request).map_err(|_| ConnectorError::Invalid)?;
        if bytes.len() > REQUEST_LIMIT {
            return Err(ConnectorError::Invalid);
        }
        let (status, body) = self
            .post("http://localhost/internal/connector", bytes)
            .await
            .map_err(|_| ConnectorError::Unavailable)?;
        classify_connector(status, &body)
    }

    async fn post(
        &self,
        entry: &'static str,
        bytes: Vec<u8>,
    ) -> Result<(StatusCode, Vec<u8>), &'static str> {
        let mut response = self
            .client
            .post(entry)
            .header("content-type", "application/json")
            .header("cf-connecting-ip", "127.0.0.1")
            .body(bytes)
            .send()
            .await
            .map_err(|_| "黑板权威服务不可用；停止协作")?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "黑板响应不可用")? {
            if body.len() + chunk.len() > RESPONSE_LIMIT {
                return Err("黑板响应超过上限");
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }
}

/// 按状态码类别给应答定性：先看状态码再看正文，4xx／5xx 带非 JSON 正文时也不混为一谈。
fn classify(status: StatusCode, body: &[u8]) -> Result<serde_json::Value, BoardCallError> {
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    if status.is_success() {
        return match parsed {
            Some(value) => Ok(value),
            None => Err(BoardCallError::Unavailable("黑板响应无效".into())),
        };
    }
    let message = parsed.as_ref().and_then(error_message);
    if status.is_client_error() {
        let message = message.unwrap_or_else(|| REJECTED.into());
        return Err(BoardCallError::Rejected(message));
    }
    // 5xx 与其余非 2xx（重定向已关）都说明权威服务这一刻给不出结论，不是请求错了。
    let message = message.unwrap_or_else(|| UNAVAILABLE.into());
    Err(BoardCallError::Unavailable(message))
}

/// 状态码与代码都对得上才算权威服务的结论；只看状态码的话，旧权威服务「没有这个入口」的 404 会被
/// 当成「没有这条工作项」。
fn classify_connector(
    status: StatusCode,
    body: &[u8],
) -> Result<serde_json::Value, ConnectorError> {
    let parsed = serde_json::from_slice::<serde_json::Value>(body).ok();
    if status.is_success() {
        return parsed.ok_or(ConnectorError::Unavailable);
    }
    let code = parsed
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(serde_json::Value::as_str);
    Err(match (status.as_u16(), code) {
        (403, Some("connector_denied")) => ConnectorError::Denied,
        (404, Some("connector_not_found")) => ConnectorError::NotFound,
        (409, Some("connector_stale")) => ConnectorError::Stale,
        (422, Some("connector_invalid")) => ConnectorError::Invalid,
        _ => ConnectorError::Unavailable,
    })
}

/// 取正文 `error` 字段的前若干字符；没有或不是字符串就是 `None`。
fn error_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(|message| message.chars().take(MESSAGE_LIMIT).collect())
}
