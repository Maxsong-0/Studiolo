//! Capability-checked tool and model-relay seam with in-memory M0 adapters.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};

/// Stable idempotency identity of a Gateway invocation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CallId(String);

impl CallId {
    /// Parses a non-empty identifier without control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(GatewayError::InvalidCallId);
        }
        Ok(Self(value))
    }

    /// Returns the textual identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for CallId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Short-lived authority presented by a runner for one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCapability {
    task_id: TaskId,
    attempt_id: AttemptId,
    fence: Fence,
    allowed_tools: PermissionSet,
    expires_at_unix_seconds: i64,
}

impl JobCapability {
    /// Constructs the M0 in-process representation of a validated capability.
    #[must_use]
    pub fn new(
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        allowed_tools: PermissionSet,
        expires_at_unix_seconds: i64,
    ) -> Self {
        Self {
            task_id,
            attempt_id,
            fence,
            allowed_tools,
            expires_at_unix_seconds,
        }
    }

    /// Returns the bound task.
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// Returns the bound attempt.
    #[must_use]
    pub fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }

    /// Returns the bound fencing token.
    #[must_use]
    pub fn fence(&self) -> Fence {
        self.fence
    }

    /// Returns the capability's tool ceiling.
    #[must_use]
    pub fn allowed_tools(&self) -> &PermissionSet {
        &self.allowed_tools
    }

    /// Returns the expiry as seconds from the Unix epoch.
    #[must_use]
    pub fn expires_at_unix_seconds(&self) -> i64 {
        self.expires_at_unix_seconds
    }
}

/// Fresh state returned by [`AttemptGuard`] for an attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttemptSnapshot {
    fence: Fence,
    cancelled: bool,
}

impl AttemptSnapshot {
    /// Creates a live attempt state.
    #[must_use]
    pub fn active(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: false,
        }
    }

    /// Creates a cancelled attempt state.
    #[must_use]
    pub fn cancelled(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: true,
        }
    }

    /// Returns the current fencing token.
    #[must_use]
    pub fn fence(self) -> Fence {
        self.fence
    }

    /// Returns whether capability use has been cancelled.
    #[must_use]
    pub fn is_cancelled(self) -> bool {
        self.cancelled
    }
}

/// Failure to load the live attempt state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttemptGuardError {
    /// The task/attempt pair is not active or does not exist.
    #[error("attempt is not active")]
    UnknownAttempt,
    /// The runtime-state authority cannot currently answer.
    #[error("attempt guard is unavailable")]
    Unavailable,
}

/// Live, non-cached authority for fence and cancellation checks.
pub trait AttemptGuard: Send + Sync {
    /// Loads current state for the exact task/attempt pair.
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError>;
}

/// Mutable in-memory attempt authority for contract and restart tests.
#[derive(Debug, Default)]
pub struct MemoryAttemptGuard {
    attempts: Mutex<BTreeMap<(TaskId, AttemptId), AttemptSnapshot>>,
}

impl MemoryAttemptGuard {
    /// Creates an empty guard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates the current state of an attempt.
    pub fn set(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
        snapshot: AttemptSnapshot,
    ) -> Result<(), AttemptGuardError> {
        self.attempts
            .lock()
            .map_err(|_| AttemptGuardError::Unavailable)?
            .insert((task_id, attempt_id), snapshot);
        Ok(())
    }
}

impl AttemptGuard for MemoryAttemptGuard {
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError> {
        self.attempts
            .lock()
            .map_err(|_| AttemptGuardError::Unavailable)?
            .get(&(task_id, attempt_id))
            .copied()
            .ok_or(AttemptGuardError::UnknownAttempt)
    }
}

/// Clock boundary used to evaluate capability expiry without sleeping in tests.
pub trait Clock: Send + Sync {
    /// Returns whole seconds from the Unix epoch.
    fn now_unix_seconds(&self) -> i64;
}

/// Deterministic clock for M0 contract tests.
#[derive(Debug)]
pub struct ManualClock {
    now: std::sync::atomic::AtomicI64,
}

impl ManualClock {
    /// Creates a clock fixed at `now_unix_seconds`.
    #[must_use]
    pub fn new(now_unix_seconds: i64) -> Self {
        Self {
            now: std::sync::atomic::AtomicI64::new(now_unix_seconds),
        }
    }

    /// Changes the current instant.
    pub fn set(&self, now_unix_seconds: i64) {
        self.now
            .store(now_unix_seconds, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Independently imposed deployment, policy, and role ceilings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionLayers {
    deployment: PermissionSet,
    policy: PermissionSet,
    role: PermissionSet,
}

impl PermissionLayers {
    /// Creates the three Gateway-owned permission ceilings.
    #[must_use]
    pub fn new(deployment: PermissionSet, policy: PermissionSet, role: PermissionSet) -> Self {
        Self {
            deployment,
            policy,
            role,
        }
    }

    fn effective(&self, capability: &JobCapability) -> PermissionSet {
        self.deployment
            .intersect(&self.policy)
            .intersect(&self.role)
            .intersect(capability.allowed_tools())
    }
}

/// Structured request for a logical Gateway tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    call_id: CallId,
    tool: ToolId,
    arguments: Value,
}

impl ToolCall {
    /// Creates a tool call. Tool-specific argument validation belongs to its adapter.
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, arguments: Value) -> Self {
        Self {
            call_id,
            tool,
            arguments,
        }
    }

    /// Returns the stable call identifier.
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Returns the requested logical tool.
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// Returns the structured tool arguments.
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

/// Structured result from a logical Gateway tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    call_id: CallId,
    tool: ToolId,
    output: Value,
}

impl ToolResult {
    /// Creates a structured tool result in a Gateway adapter.
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, output: Value) -> Self {
        Self {
            call_id,
            tool,
            output,
        }
    }

    /// Returns the stable call identifier.
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Returns the logical tool.
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// Returns the in-memory adapter output.
    #[must_use]
    pub fn output(&self) -> &Value {
        &self.output
    }
}

/// Request for the model-relay branch of the Gateway seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    call_id: CallId,
    provider: String,
    model: String,
    prompt: String,
}

impl ModelRequest {
    /// Creates a relay request.
    #[must_use]
    pub fn new(
        call_id: CallId,
        provider: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            call_id,
            provider: provider.into(),
            model: model.into(),
            prompt: prompt.into(),
        }
    }

    /// Returns the stable call identifier.
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Returns the logical provider name.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the logical model name.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the relay prompt supplied by the caller.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

/// Result from the model-relay branch of the Gateway seam.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelResponse {
    call_id: CallId,
    provider: String,
    model: String,
    text: String,
}

impl ModelResponse {
    /// Creates a structured model response in a Gateway adapter.
    #[must_use]
    pub fn new(
        call_id: CallId,
        provider: impl Into<String>,
        model: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            call_id,
            provider: provider.into(),
            model: model.into(),
            text: text.into(),
        }
    }

    /// Returns the stable call identifier.
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// Returns the configured provider.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the configured model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the relay response text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Errors returned at the Gateway boundary.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    /// A call identifier was empty or contained control characters.
    #[error("gateway call id is invalid")]
    InvalidCallId,
    /// The task/attempt pair is unknown.
    #[error("attempt is not active")]
    AttemptNotActive,
    /// Runtime state could not be checked, so the request failed closed.
    #[error("attempt guard is unavailable")]
    AttemptGuardUnavailable,
    /// The attempt was cancelled and its capability is revoked.
    #[error("attempt is cancelled")]
    Cancelled,
    /// The presented fencing token is stale.
    #[error("capability fence does not match the live attempt")]
    FenceMismatch,
    /// The short-lived capability has expired.
    #[error("job capability is expired")]
    CapabilityExpired,
    /// At least one permission layer denied the requested tool.
    #[error("tool is not allowed")]
    ToolNotAllowed,
    /// Authoritative audit persistence failed, so the call was not released.
    #[error("authoritative audit is unavailable")]
    AuditUnavailable,
    /// The in-memory tool adapter has no configured response.
    #[error("tool adapter is unavailable")]
    ToolUnavailable,
    /// The in-memory model adapter has no configured response.
    #[error("model relay is unavailable")]
    ModelUnavailable,
    /// A call identifier was reused with different request content.
    #[error("call id conflicts with a prior request")]
    CallConflict,
    /// In-memory adapter state was poisoned.
    #[error("gateway state is unavailable")]
    StateUnavailable,
}

impl GatewayError {
    fn audit_reason(&self) -> &'static str {
        match self {
            Self::InvalidCallId => "invalid_call_id",
            Self::AttemptNotActive => "attempt_not_active",
            Self::AttemptGuardUnavailable => "attempt_guard_unavailable",
            Self::Cancelled => "cancelled",
            Self::FenceMismatch => "fence_mismatch",
            Self::CapabilityExpired => "capability_expired",
            Self::ToolNotAllowed => "tool_not_allowed",
            Self::AuditUnavailable => "audit_unavailable",
            Self::ToolUnavailable => "tool_unavailable",
            Self::ModelUnavailable => "model_unavailable",
            Self::CallConflict => "call_conflict",
            Self::StateUnavailable => "state_unavailable",
        }
    }
}

/// Synchronous, object-safe Gateway seam used by Stub and future real runners.
pub trait Gateway: Send + Sync {
    /// Invokes one capability-checked logical tool.
    fn invoke(
        &self,
        capability: &JobCapability,
        call: ToolCall,
    ) -> Result<ToolResult, GatewayError>;

    /// Relays one capability-checked model request.
    fn relay(
        &self,
        capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError>;
}

/// In-memory Gateway adapter with configurable deterministic responses.
pub struct MemoryGateway {
    guard: Arc<dyn AttemptGuard>,
    audit: Arc<dyn AuditLedger>,
    clock: Arc<dyn Clock>,
    permissions: PermissionLayers,
    tool_responses: Mutex<BTreeMap<ToolId, Value>>,
    model_responses: Mutex<BTreeMap<(String, String), String>>,
    tool_calls: Mutex<BTreeMap<(AttemptId, CallId), (ToolCall, ToolResult)>>,
    model_calls: Mutex<BTreeMap<(AttemptId, CallId), (ModelRequest, ModelResponse)>>,
}

impl MemoryGateway {
    /// Creates an adapter from explicit runtime-state, audit, clock, and policy seams.
    #[must_use]
    pub fn new(
        guard: Arc<dyn AttemptGuard>,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn Clock>,
        permissions: PermissionLayers,
    ) -> Self {
        Self {
            guard,
            audit,
            clock,
            permissions,
            tool_responses: Mutex::new(BTreeMap::new()),
            model_responses: Mutex::new(BTreeMap::new()),
            tool_calls: Mutex::new(BTreeMap::new()),
            model_calls: Mutex::new(BTreeMap::new()),
        }
    }

    /// Configures the deterministic result for one logical tool.
    pub fn set_tool_response(&self, tool: ToolId, output: Value) -> Result<(), GatewayError> {
        self.lock(&self.tool_responses)?.insert(tool, output);
        Ok(())
    }

    /// Configures the deterministic result for one provider/model pair.
    pub fn set_model_response(
        &self,
        provider: impl Into<String>,
        model: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<(), GatewayError> {
        self.lock(&self.model_responses)?
            .insert((provider.into(), model.into()), text.into());
        Ok(())
    }

    fn lock<'a, T>(&self, mutex: &'a Mutex<T>) -> Result<MutexGuard<'a, T>, GatewayError> {
        mutex.lock().map_err(|_| GatewayError::StateUnavailable)
    }

    fn audit_event_id(
        &self,
        domain: &str,
        capability: &JobCapability,
        call_id: &CallId,
    ) -> Result<AuditEventId, GatewayError> {
        AuditEventId::parse(format!(
            "{domain}:task={}:attempt={}:fence={}:call={}:{}",
            capability.task_id(),
            capability.attempt_id(),
            capability.fence().get(),
            call_id.as_str().len(),
            call_id,
        ))
        .map_err(|_| GatewayError::InvalidCallId)
    }

    fn recheck_audit_event_id(
        &self,
        base: &AuditEventId,
        error: &GatewayError,
    ) -> Result<AuditEventId, GatewayError> {
        AuditEventId::parse(format!("{base}:recheck:{}", error.audit_reason()))
            .map_err(|_| GatewayError::InvalidCallId)
    }

    fn inspect(&self, capability: &JobCapability) -> Result<AttemptSnapshot, GatewayError> {
        self.guard
            .inspect(capability.task_id(), capability.attempt_id())
            .map_err(|error| match error {
                AttemptGuardError::UnknownAttempt => GatewayError::AttemptNotActive,
                AttemptGuardError::Unavailable => GatewayError::AttemptGuardUnavailable,
            })
    }

    fn authorize(&self, capability: &JobCapability, tool: &ToolId) -> Result<(), GatewayError> {
        let live = self.inspect(capability)?;
        if live.is_cancelled() {
            return Err(GatewayError::Cancelled);
        }
        if live.fence() != capability.fence() {
            return Err(GatewayError::FenceMismatch);
        }
        if self.clock.now_unix_seconds() >= capability.expires_at_unix_seconds() {
            return Err(GatewayError::CapabilityExpired);
        }
        if !self.permissions.effective(capability).allows(tool) {
            return Err(GatewayError::ToolNotAllowed);
        }
        Ok(())
    }

    fn append_audit(
        &self,
        event_id: AuditEventId,
        phase: AuditPhase,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        payload: Value,
    ) -> Result<(), GatewayError> {
        let payload =
            SanitizedPayload::try_from(payload).map_err(|_| GatewayError::AuditUnavailable)?;
        let subject = AuditSubject::default()
            .with_task(capability.task_id())
            .with_attempt(capability.attempt_id())
            .with_tool(tool);
        self.audit
            .append(AuditEvent::new(event_id, phase, action, payload).with_subject(subject))
            .map(|_| ())
            .map_err(|_| GatewayError::AuditUnavailable)
    }

    fn audit_denial(
        &self,
        event_id: AuditEventId,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        call_id: &CallId,
        error: GatewayError,
    ) -> GatewayError {
        let audit_result = self.append_audit(
            event_id,
            AuditPhase::Outcome,
            action,
            capability,
            tool.clone(),
            json!({
                "call_id": call_id.as_str(),
                "tool": tool.as_str(),
                "fence": capability.fence().get(),
                "decision": "deny",
                "reason": error.audit_reason(),
            }),
        );
        audit_result.err().unwrap_or(error)
    }
}

impl Gateway for MemoryGateway {
    fn invoke(
        &self,
        capability: &JobCapability,
        call: ToolCall,
    ) -> Result<ToolResult, GatewayError> {
        let call_key = (capability.attempt_id(), call.call_id().clone());
        let audit_id = self.audit_event_id("gateway.tool.v1", capability, call.call_id())?;
        if let Err(error) = self.authorize(capability, call.tool()) {
            let audit_id = if self.lock(&self.tool_calls)?.contains_key(&call_key) {
                self.recheck_audit_event_id(&audit_id, &error)?
            } else {
                audit_id
            };
            return Err(self.audit_denial(
                audit_id,
                "gateway.tool_call",
                capability,
                call.tool().clone(),
                call.call_id(),
                error,
            ));
        }

        if let Some((prior_call, result)) = self.lock(&self.tool_calls)?.get(&call_key) {
            return if prior_call == &call {
                Ok(result.clone())
            } else {
                let error = GatewayError::CallConflict;
                let conflict_id = self.recheck_audit_event_id(&audit_id, &error)?;
                Err(self.audit_denial(
                    conflict_id,
                    "gateway.tool_call",
                    capability,
                    call.tool().clone(),
                    call.call_id(),
                    error,
                ))
            };
        }

        self.append_audit(
            audit_id.clone(),
            AuditPhase::Intent,
            "gateway.tool_call",
            capability,
            call.tool().clone(),
            json!({
                "call_id": call.call_id().as_str(),
                "tool": call.tool().as_str(),
                "fence": capability.fence().get(),
                "decision": "pending",
            }),
        )?;

        let output = match self.lock(&self.tool_responses)?.get(call.tool()).cloned() {
            Some(output) => output,
            None => {
                let error = GatewayError::ToolUnavailable;
                self.append_audit(
                    audit_id,
                    AuditPhase::Outcome,
                    "gateway.tool_call",
                    capability,
                    call.tool().clone(),
                    json!({
                        "call_id": call.call_id().as_str(),
                        "tool": call.tool().as_str(),
                        "fence": capability.fence().get(),
                        "decision": "error",
                        "reason": error.audit_reason(),
                    }),
                )?;
                return Err(error);
            }
        };
        let result = ToolResult {
            call_id: call.call_id().clone(),
            tool: call.tool().clone(),
            output,
        };
        self.append_audit(
            audit_id,
            AuditPhase::Outcome,
            "gateway.tool_call",
            capability,
            call.tool().clone(),
            json!({
                "call_id": call.call_id().as_str(),
                "tool": call.tool().as_str(),
                "fence": capability.fence().get(),
                "decision": "allow",
            }),
        )?;
        self.lock(&self.tool_calls)?
            .insert(call_key, (call, result.clone()));
        Ok(result)
    }

    fn relay(
        &self,
        capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError> {
        let relay_tool = ToolId::from_static("model.relay");
        let call_key = (capability.attempt_id(), request.call_id.clone());
        let audit_id = self.audit_event_id("gateway.model.v1", capability, &request.call_id)?;
        if let Err(error) = self.authorize(capability, &relay_tool) {
            let audit_id = if self.lock(&self.model_calls)?.contains_key(&call_key) {
                self.recheck_audit_event_id(&audit_id, &error)?
            } else {
                audit_id
            };
            return Err(self.audit_denial(
                audit_id,
                "gateway.model_relay",
                capability,
                relay_tool,
                &request.call_id,
                error,
            ));
        }

        if let Some((prior_request, response)) = self.lock(&self.model_calls)?.get(&call_key) {
            return if prior_request == &request {
                Ok(response.clone())
            } else {
                let error = GatewayError::CallConflict;
                let conflict_id = self.recheck_audit_event_id(&audit_id, &error)?;
                Err(self.audit_denial(
                    conflict_id,
                    "gateway.model_relay",
                    capability,
                    relay_tool,
                    &request.call_id,
                    error,
                ))
            };
        }

        self.append_audit(
            audit_id.clone(),
            AuditPhase::Intent,
            "gateway.model_relay",
            capability,
            relay_tool.clone(),
            json!({
                "call_id": request.call_id.as_str(),
                "tool": relay_tool.as_str(),
                "provider": request.provider,
                "model": request.model,
                "fence": capability.fence().get(),
                "decision": "pending",
            }),
        )?;
        let text = match self
            .lock(&self.model_responses)?
            .get(&(request.provider.clone(), request.model.clone()))
            .cloned()
        {
            Some(text) => text,
            None => {
                let error = GatewayError::ModelUnavailable;
                self.append_audit(
                    audit_id,
                    AuditPhase::Outcome,
                    "gateway.model_relay",
                    capability,
                    relay_tool,
                    json!({
                        "call_id": request.call_id.as_str(),
                        "tool": "model.relay",
                        "provider": request.provider,
                        "model": request.model,
                        "fence": capability.fence().get(),
                        "decision": "error",
                        "reason": error.audit_reason(),
                    }),
                )?;
                return Err(error);
            }
        };
        let response = ModelResponse {
            call_id: request.call_id.clone(),
            provider: request.provider.clone(),
            model: request.model.clone(),
            text,
        };
        self.append_audit(
            audit_id,
            AuditPhase::Outcome,
            "gateway.model_relay",
            capability,
            relay_tool.clone(),
            json!({
                "call_id": request.call_id.as_str(),
                "tool": relay_tool.as_str(),
                "provider": request.provider,
                "model": request.model,
                "fence": capability.fence().get(),
                "decision": "allow",
            }),
        )?;
        self.lock(&self.model_calls)?
            .insert(call_key, (request, response.clone()));
        Ok(response)
    }
}
