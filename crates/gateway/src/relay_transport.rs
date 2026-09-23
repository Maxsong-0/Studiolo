//! 固定主机的 TLS 传输：不跟随重定向、不走代理，SSE 接收有上限。
use crate::relay_types::{ProviderCredential, RelayError, RelayGrant, RelayProvider};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// 可注入的网络边界，供确定性的失败／结果未定契约测试使用。
pub trait RelayTransport: Send + Sync {
    /// 发送一个已授权的请求。实现不得重试结果未知的请求。
    fn execute(
        &self,
        grant: &RelayGrant,
        request: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError>;
}
/// 唯一同时持有两份专用供应商凭据的组件。
pub struct HttpsRelayTransport {
    anthropic: Arc<ProviderCredential>,
    openai: Arc<ProviderCredential>,
}
impl HttpsRelayTransport {
    /// 凭据必须来自仅 Gateway 进程可见、彼此独立的密钥挂载。
    #[must_use]
    pub fn new(anthropic: ProviderCredential, openai: ProviderCredential) -> Self {
        Self {
            anthropic: Arc::new(anthropic),
            openai: Arc::new(openai),
        }
    }
}
impl HttpsRelayTransport {
    /// 只把分块流式交给可信的 Gateway 前端进程；这不是向 Runner 发布。
    pub fn forward(
        &self,
        grant: &RelayGrant,
        request: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
        chunk_sink: &mut dyn FnMut(&[u8]) -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| RelayError::Upstream)?;
        runtime.block_on(async {
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(u64::from(grant.limits.timeout_seconds)))
                .build()
                .map_err(|_| RelayError::Upstream)?;
            let request = client
                .post(grant.provider.url())
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .body(request.to_vec());
            // 不转发任何来自 CLI 的请求头；专用凭据只在此处注入。
            let request = match grant.provider {
                RelayProvider::Anthropic => request
                    .header("x-api-key", self.anthropic.expose())
                    .header("anthropic-version", "2023-06-01"),
                RelayProvider::OpenAi => request.bearer_auth(self.openai.expose()),
            };
            check()?;
            let mut response = request.send().await.map_err(|_| RelayError::Upstream)?;
            if !response.status().is_success() {
                return Err(RelayError::Upstream);
            }
            let media = response
                .headers()
                .get("content-type")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            if !media.starts_with("text/event-stream") {
                return Err(RelayError::Upstream);
            }
            let mut output = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| RelayError::Upstream)? {
                check()?;
                if output.len().saturating_add(chunk.len()) > grant.limits.response_bytes as usize {
                    return Err(RelayError::Upstream);
                }
                chunk_sink(&chunk)?;
                output.extend_from_slice(&chunk);
            }
            check()?;
            validate_terminal(grant.provider, &output)?;
            Ok(output)
        })
    }
}
impl RelayTransport for HttpsRelayTransport {
    fn execute(
        &self,
        grant: &RelayGrant,
        request: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError> {
        self.forward(grant, request, check, &mut |_| Ok(()))
    }
}
pub(crate) fn validate_terminal(provider: RelayProvider, bytes: &[u8]) -> Result<(), RelayError> {
    let text = std::str::from_utf8(bytes).map_err(|_| RelayError::Upstream)?;
    let expected = match provider {
        RelayProvider::Anthropic => "message_stop",
        RelayProvider::OpenAi => "response.completed",
    };
    let mut terminal = false;
    for line in text.lines().filter_map(|line| line.strip_prefix("data:")) {
        if line.trim() == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(line.trim()).map_err(|_| RelayError::Upstream)?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(RelayError::Upstream)?;
        if matches!(
            event_type,
            "error" | "response.failed" | "response.incomplete"
        ) {
            return Err(RelayError::Upstream);
        }
        if event_type == expected {
            terminal = true;
        }
    }
    if terminal {
        Ok(())
    } else {
        Err(RelayError::Upstream)
    }
}
