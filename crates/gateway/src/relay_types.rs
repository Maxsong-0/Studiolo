//! Typed real-relay policy, credentials and publication boundaries.
use crate::{CallId, JobCapability};
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};
use zeroize::Zeroizing;

/// Fixed upstream selected only by the authenticated scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayProvider {
    /// Anthropic Messages SSE.
    Anthropic,
    /// OpenAI Responses SSE.
    OpenAi,
}
impl RelayProvider {
    pub(crate) fn url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1/messages",
            Self::OpenAi => "https://api.openai.com/v1/responses",
        }
    }
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Anthropic => "/v1/messages",
            Self::OpenAi => "/v1/responses",
        }
    }
}
/// Authenticated data classification; missing classification is deny.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEgress {
    /// Stub only.
    #[default]
    Deny,
    /// Stub only because no local model is shipped.
    LocalOnly,
    /// Exact provider allowlist.
    Providers(Vec<RelayProvider>),
}
impl ModelEgress {
    pub(crate) fn allows(&self, provider: RelayProvider) -> bool {
        matches!(self, Self::Providers(providers) if providers.contains(&provider))
    }
}
/// Immutable classification of one captured source revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelaySource {
    /// Logical source identity.
    pub source: String,
    /// Captured revision.
    pub revision: String,
    /// SHA-256 of captured bytes.
    pub sha256: String,
    /// Source retention bound; raw relay output may never outlive it.
    pub retain_until_unix_seconds: i64,
    /// Authenticated effective egress label.
    #[serde(default)]
    pub model_egress: ModelEgress,
}
/// Hard ceilings; money is in millionths of a USD.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayLimits {
    /// Maximum request bytes; also bounds worst-case input tokens conservatively.
    pub request_bytes: u32,
    /// Maximum aggregate SSE response bytes.
    pub response_bytes: u32,
    /// Upstream maximum response tokens.
    pub response_tokens: u32,
    /// Maximum total calls for the attempt.
    pub calls: u32,
    /// First prompt pacing threshold.
    pub pacing_tools: u32,
    /// Prompt wrapup threshold.
    pub wrapup_tools: u32,
    /// Hard model-emitted tool-call ceiling, enforced before returning each response.
    pub max_tools: u32,
    /// Attempt spend ceiling.
    pub budget_microusd: u64,
    /// Trusted worst-case charge per request including tool/schema input overhead.
    pub reservation_microusd: u64,
    /// Admin-pinned conservative upper input-token rate for this exact model.
    pub input_token_microusd: u64,
    /// Admin-pinned conservative upper output-token rate for this exact model.
    pub output_token_microusd: u64,
    /// Maximum wall-clock duration of a single upstream request.
    pub timeout_seconds: u32,
}
/// Trusted scheduler grant, never accepted from the Runner's request body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayGrant {
    /// Existing four-layer capability ceiling and attempt identity.
    pub capability: JobCapability,
    /// A separate audience from tool invocation.
    pub audience: String,
    /// Immutable snapshot binding.
    pub snapshot_id: String,
    /// Current effective policy digest.
    pub policy_hash: String,
    /// Scheduler-owned role identity.
    pub role: String,
    /// Exact CLI identity.
    pub adapter: String,
    /// Fixed upstream.
    pub provider: RelayProvider,
    /// Exact model.
    pub model: String,
    /// Every source in the snapshot; no Runner-supplied overrides.
    pub sources: Vec<RelaySource>,
    /// Hard ceilings.
    pub limits: RelayLimits,
}
/// Redacted, non-serializable short-lived bearer; only explicit exposure sends it to an attempt.
pub struct RelayTicket(Zeroizing<String>);
impl RelayTicket {
    pub(crate) fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    /// Exposes a short-lived job capability for delivery via a private ephemeral file.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for RelayTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RelayTicket([redacted])")
    }
}
/// Gateway-only dedicated provider key. Never serializable or included in diagnostics.
pub struct ProviderCredential(Zeroizing<String>);
impl ProviderCredential {
    /// Loads a dedicated key supplied by the process-specific secret mount.
    pub fn new(value: String) -> Result<Self, RelayError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(RelayError::Credential);
        }
        Ok(Self(Zeroizing::new(value)))
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for ProviderCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProviderCredential([redacted])")
    }
}
/// Trusted current policy/lease authority, evaluated for every boundary including replays.
pub trait RelayPolicyGuard: Send + Sync {
    /// Rechecks the snapshot classification and effective policy hash against current authority.
    fn inspect(&self, grant: &RelayGrant, now_seconds: i64) -> Result<(), RelayError>;
}
/// Broker publication gate; a successful return means its manifest marker is durable.
pub trait RelayPublisher: Send + Sync {
    /// Classifies and seals the ephemeral response through the authoritative artifact path.
    /// Must be idempotent by `(attempt, call)` and reject source-label changes.
    fn seal(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        raw: &Path,
        sha256: &str,
    ) -> Result<(), RelayError>;
}
/// Bounded response; real upstream authentication never reaches this object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayResponse {
    /// Protocol media type.
    pub content_type: &'static str,
    /// SSE bytes, released only after durable outcome and manifest publication.
    pub body: Vec<u8>,
}
/// Stable errors; no upstream body, URL query, prompt or key appears in Display/Debug.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RelayError {
    /// Capability or authoritative binding is invalid.
    #[error("relay authorization denied")]
    Denied,
    /// At least one source forbids this provider.
    #[error("model egress denied")]
    Egress,
    /// Admission freeze is active.
    #[error("gateway admission frozen")]
    Frozen,
    /// Hard reservation cannot be funded.
    #[error("relay budget exhausted")]
    Budget,
    /// Request shape or bounds do not match the pinned contract.
    #[error("relay request rejected")]
    Request,
    /// Persistent reservation/result store unavailable.
    #[error("relay durable state unavailable")]
    Storage,
    /// Same call id is bound to a different request.
    #[error("relay call id conflict")]
    Conflict,
    /// An upstream side effect may have occurred; automatic replay is prohibited.
    #[error("relay call indeterminate; explicit operator resolution required")]
    Indeterminate,
    /// Upstream failed or exceeded protocol/size/time constraints.
    #[error("relay upstream unavailable")]
    Upstream,
    /// Required authoritative audit append failed.
    #[error("relay audit unavailable")]
    Audit,
    /// Broker could not seal the artifact.
    #[error("relay artifact publication unavailable")]
    Publication,
    /// Dedicated provider credential unavailable.
    #[error("dedicated provider credential unavailable")]
    Credential,
    /// Retained response expired; never repeat the external request.
    #[error("relay result retention expired")]
    Expired,
}
