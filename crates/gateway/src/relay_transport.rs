//! Fixed-host TLS transport with no redirects/proxies and bounded SSE ingestion.
use crate::relay_types::{ProviderCredential, RelayError, RelayGrant, RelayProvider};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// Network boundary injectable for deterministic failure/indeterminate contract tests.
pub trait RelayTransport: Send + Sync {
    /// Sends one already-authorized request. Implementations must not retry an unknown outcome.
    fn execute(
        &self,
        grant: &RelayGrant,
        request: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError>;
}
/// The only component that holds both dedicated provider credentials.
pub struct HttpsRelayTransport {
    anthropic: Arc<ProviderCredential>,
    openai: Arc<ProviderCredential>,
}
impl HttpsRelayTransport {
    /// Credentials must originate from separate Gateway-only process secret mounts.
    #[must_use]
    pub fn new(anthropic: ProviderCredential, openai: ProviderCredential) -> Self {
        Self {
            anthropic: Arc::new(anthropic),
            openai: Arc::new(openai),
        }
    }
}
impl HttpsRelayTransport {
    /// Streams chunks only to the trusted Gateway front process; this is not Runner publication.
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
            // No incoming CLI headers are forwarded. Dedicated credentials are injected only here.
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
