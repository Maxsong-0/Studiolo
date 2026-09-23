//! 经 capability 校验的工具与模型中继接缝：含 M0 内存适配器与持久化真实中继 `DurableRelay`。

mod admission;
mod durable_relay;
#[cfg(unix)]
mod relay_remote;
mod relay_request;
mod relay_retention;
mod relay_store;
mod relay_tools;
mod relay_transport;
mod relay_types;
mod relay_unix;
#[cfg(unix)]
pub use relay_remote::{RemoteRelayTransport, serve_key_holder_connection};

pub use durable_relay::{DurableRelay, RelayServices};
pub use relay_transport::{HttpsRelayTransport, RelayTransport};
pub use relay_types::{
    ModelEgress, ProviderCredential, RelayError, RelayGrant, RelayLimits, RelayPolicyGuard,
    RelayProvider, RelayPublisher, RelayResponse, RelaySource, RelayTicket,
};
#[cfg(unix)]
pub use relay_unix::serve_relay_connection;
#[cfg(test)]
mod wait_probe;
#[cfg(test)]
mod waiting_tests;

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};

/// Gateway 调用的稳定幂等身份。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CallId(String);

impl CallId {
    /// 解析标识符：不能为空或全为空白，也不能含控制字符。
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(GatewayError::InvalidCallId);
        }
        Ok(Self(value))
    }

    /// 返回标识符文本。
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

/// runner 为某一次 attempt 出示的短期授权凭证。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCapability {
    task_id: TaskId,
    attempt_id: AttemptId,
    fence: Fence,
    allowed_tools: PermissionSet,
    expires_at_unix_seconds: i64,
}

impl JobCapability {
    /// 构造已校验 capability 在 M0 进程内的表示。
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

    /// 返回绑定的任务。
    #[must_use]
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// 返回绑定的 attempt。
    #[must_use]
    pub fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }

    /// 返回绑定的 fencing token。
    #[must_use]
    pub fn fence(&self) -> Fence {
        self.fence
    }

    /// 返回该 capability 的工具上限。
    #[must_use]
    pub fn allowed_tools(&self) -> &PermissionSet {
        &self.allowed_tools
    }

    /// 返回到期时间（自 Unix 纪元起的秒数）。
    #[must_use]
    pub fn expires_at_unix_seconds(&self) -> i64 {
        self.expires_at_unix_seconds
    }
}

/// [`AttemptGuard`] 为某次 attempt 返回的最新状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttemptSnapshot {
    fence: Fence,
    cancelled: bool,
}

impl AttemptSnapshot {
    /// 创建活动中的 attempt 状态。
    #[must_use]
    pub fn active(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: false,
        }
    }

    /// 创建已取消的 attempt 状态。
    #[must_use]
    pub fn cancelled(fence: Fence) -> Self {
        Self {
            fence,
            cancelled: true,
        }
    }

    /// 返回当前的 fencing token。
    #[must_use]
    pub fn fence(self) -> Fence {
        self.fence
    }

    /// 返回 capability 的使用是否已被取消。
    #[must_use]
    pub fn is_cancelled(self) -> bool {
        self.cancelled
    }
}

/// 加载实时 attempt 状态失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttemptGuardError {
    /// 该 task/attempt 组合未处于活动状态或不存在。
    #[error("attempt is not active")]
    UnknownAttempt,
    /// 运行时状态的权威来源当前无法应答。
    #[error("attempt guard is unavailable")]
    Unavailable,
}

/// 用于 fence 与取消检查的实时、无缓存权威来源。
pub trait AttemptGuard: Send + Sync {
    /// 加载精确 task/attempt 组合的当前状态。
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError>;
}

/// 可变的内存 attempt 权威来源，供契约测试与重启测试使用。
#[derive(Debug, Default)]
pub struct MemoryAttemptGuard {
    attempts: Mutex<BTreeMap<(TaskId, AttemptId), AttemptSnapshot>>,
}

impl MemoryAttemptGuard {
    /// 创建一个空的 guard。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入或更新某次 attempt 的当前状态。
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

/// 时钟边界：用于评估 capability 是否到期，使测试无需 sleep。
pub trait Clock: Send + Sync {
    /// 返回自 Unix 纪元起的整秒数。
    fn now_unix_seconds(&self) -> i64;
}

/// 供 M0 契约测试使用的确定性时钟。
#[derive(Debug)]
pub struct ManualClock {
    now: std::sync::atomic::AtomicI64,
}

impl ManualClock {
    /// 创建固定在 `now_unix_seconds` 的时钟。
    #[must_use]
    pub fn new(now_unix_seconds: i64) -> Self {
        Self {
            now: std::sync::atomic::AtomicI64::new(now_unix_seconds),
        }
    }

    /// 修改当前时刻。
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

/// 相互独立施加的部署、策略与角色三层上限。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionLayers {
    deployment: PermissionSet,
    policy: PermissionSet,
    role: PermissionSet,
}

impl PermissionLayers {
    /// 创建 Gateway 自有的三层权限上限。
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

/// 调用 Gateway 逻辑工具的结构化请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    call_id: CallId,
    tool: ToolId,
    arguments: Value,
}

impl ToolCall {
    /// 创建一次工具调用；各工具的参数校验由其适配器负责。
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, arguments: Value) -> Self {
        Self {
            call_id,
            tool,
            arguments,
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回所请求的逻辑工具。
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// 返回结构化的工具参数。
    #[must_use]
    pub fn arguments(&self) -> &Value {
        &self.arguments
    }
}

/// Gateway 逻辑工具返回的结构化结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    call_id: CallId,
    tool: ToolId,
    output: Value,
}

impl ToolResult {
    /// 在 Gateway 适配器中创建结构化的工具结果。
    #[must_use]
    pub fn new(call_id: CallId, tool: ToolId, output: Value) -> Self {
        Self {
            call_id,
            tool,
            output,
        }
    }

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回逻辑工具。
    #[must_use]
    pub fn tool(&self) -> &ToolId {
        &self.tool
    }

    /// 返回内存适配器的输出。
    #[must_use]
    pub fn output(&self) -> &Value {
        &self.output
    }
}

/// Gateway 接缝中模型中继分支的请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    call_id: CallId,
    provider: String,
    model: String,
    prompt: String,
}

impl ModelRequest {
    /// 创建一个中继请求。
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

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回逻辑供应商名。
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// 返回逻辑模型名。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回调用方提供的中继提示词。
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

/// Gateway 接缝中模型中继分支的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelResponse {
    call_id: CallId,
    provider: String,
    model: String,
    text: String,
}

impl ModelResponse {
    /// 在 Gateway 适配器中创建结构化的模型响应。
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

    /// 返回稳定的调用标识符。
    #[must_use]
    pub fn call_id(&self) -> &CallId {
        &self.call_id
    }

    /// 返回配置的供应商。
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// 返回配置的模型。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回中继响应文本。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Gateway 边界返回的错误。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    /// 调用标识符无效：为空（或全为空白）、含控制字符，或无法据此构成审计事件 ID。
    #[error("gateway call id is invalid")]
    InvalidCallId,
    /// 该 task/attempt 组合未处于活动状态或不存在。
    #[error("attempt is not active")]
    AttemptNotActive,
    /// 无法检查运行时状态，请求按失败关闭处理。
    #[error("attempt guard is unavailable")]
    AttemptGuardUnavailable,
    /// 该 attempt 已取消，其 capability 已被撤销。
    #[error("attempt is cancelled")]
    Cancelled,
    /// 出示的 fencing token 已过时。
    #[error("capability fence does not match the live attempt")]
    FenceMismatch,
    /// 短期 capability 已过期。
    #[error("job capability is expired")]
    CapabilityExpired,
    /// 至少有一层权限拒绝了所请求的工具。
    #[error("tool is not allowed")]
    ToolNotAllowed,
    /// 权威审计持久化失败，因此调用未被放行。
    #[error("authoritative audit is unavailable")]
    AuditUnavailable,
    /// 内存工具适配器没有配置响应。
    #[error("tool adapter is unavailable")]
    ToolUnavailable,
    /// 内存模型适配器没有配置响应。
    #[error("model relay is unavailable")]
    ModelUnavailable,
    /// 同一调用标识符被用于内容不同的请求。
    #[error("call id conflicts with a prior request")]
    CallConflict,
    /// 完全相同的调用已经完成，但保留的结果已被逐出。
    #[error("call already completed and its result is no longer retained")]
    CallResultEvicted,
    /// 内存适配器状态不可用（互斥锁中毒等）。
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
            Self::CallResultEvicted => "call_result_evicted",
            Self::StateUnavailable => "state_unavailable",
        }
    }
}

/// 同步、对象安全的 Gateway 接缝，供 Stub 以及后续的真实 runner 使用。
pub trait Gateway: Send + Sync {
    /// 调用一个经 capability 校验的逻辑工具。
    fn invoke(
        &self,
        capability: &JobCapability,
        call: ToolCall,
    ) -> Result<ToolResult, GatewayError>;

    /// 中继一个经 capability 校验的模型请求。
    fn relay(
        &self,
        capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError>;
}

/// 调用幂等槽：`InFlight` 表示同键调用正在执行，`Done` 才携带可重放的结果（成功或已记入
/// 账本的失败终态）；`Evicted` 只保留请求身份，结果已按容量逐出，同请求重放得到结构化拒绝。
enum CallSlot<Request, Response> {
    InFlight,
    Done {
        request: Request,
        response: Response,
        sequence: u64,
    },
    Evicted(Request),
}

/// 同键调用的准入结论。
enum CallAdmission<Response> {
    /// 首次调用，已占位；调用方持有 [`SlotGuard`]，以 `complete` 或丢弃守卫收尾。
    Proceed,
    /// 与首次请求完全相同的重放：完整重验授权后才能返回首次结果。
    Replay(Response),
    /// 已完成的是不同请求；重新授权后才报告内容冲突。
    Conflict,
    /// 同请求已完成但结果已逐出；重新授权后报告 `CallResultEvicted`。
    Evicted,
}

/// `admit` 在锁内做出的判定；先判定再动作，避免在 `BTreeMap` 借用存活时修改它。
enum SlotDecision<Response> {
    Vacant,
    Replay(Response),
    Conflict,
    Evicted,
    Wait,
}

/// 幂等表最多保留的已完成结果数；超出后按完成顺序把最早的结果逐出为 `Evicted`。
/// 结果正文（arguments / output / prompt / text）随逐出释放，不随进程寿命无界驻留。
pub const MAX_RETAINED_RESULTS: usize = 4096;

/// 一次已占位执行的结局：`Recorded` 表示账本已写入终局 `Outcome`（成功或失败），该
/// call ID 从此封口；`Unrecorded` 表示没有任何 `Outcome` 落账，占位应归还以允许重试。
enum Execution<Response> {
    Recorded(Result<Response, GatewayError>),
    Unrecorded(GatewayError),
}

/// 按 `(attempt, call)` 键串行化同一调用：并发同键请求只执行一次，其余等待首次结果后再判定
/// 重放或冲突。检查与占位在同一把锁内完成，不存在「都通过检查、各执行一次」的窗口。
struct CallTable<Request, Response> {
    slots: Mutex<BTreeMap<(AttemptId, CallId), CallSlot<Request, Response>>>,
    finished: Condvar,
    next_sequence: AtomicU64,
    #[cfg(test)]
    wait_probe: wait_probe::WaitProbe,
}

/// 占位守卫：取得 `Proceed` 的调用无论正常返回、提前返回还是 panic 展开，都归还自己的
/// 占位并唤醒等待者；只有显式 `complete` 才把占位替换为可重放的结果。
struct SlotGuard<'a, Request: PartialEq, Response: Clone> {
    table: &'a CallTable<Request, Response>,
    key: Option<(AttemptId, CallId)>,
}

impl<'a, Request: PartialEq, Response: Clone> SlotGuard<'a, Request, Response> {
    fn new(table: &'a CallTable<Request, Response>, key: (AttemptId, CallId)) -> Self {
        Self {
            table,
            key: Some(key),
        }
    }

    fn complete(mut self, request: Request, response: Response) -> Result<(), GatewayError> {
        let key = self.key.take().ok_or(GatewayError::StateUnavailable)?;
        self.table.complete(key, request, response)
    }
}

impl<Request: PartialEq, Response: Clone> Drop for SlotGuard<'_, Request, Response> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            // 展开或提前返回时锁不可用也无法再做别的；等待者由 abandon 内的 notify_all 唤醒。
            let _ = self.table.abandon(&key);
        }
    }
}

impl<Request: PartialEq, Response: Clone> CallTable<Request, Response> {
    fn new() -> Self {
        Self {
            slots: Mutex::new(BTreeMap::new()),
            finished: Condvar::new(),
            next_sequence: AtomicU64::new(0),
            #[cfg(test)]
            wait_probe: wait_probe::WaitProbe::default(),
        }
    }

    fn admit(
        &self,
        key: (AttemptId, CallId),
        request: &Request,
    ) -> Result<CallAdmission<Response>, GatewayError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        loop {
            let decision = match slots.get(&key) {
                None => SlotDecision::Vacant,
                Some(CallSlot::Done {
                    request: prior,
                    response,
                    ..
                }) if prior == request => SlotDecision::Replay(response.clone()),
                Some(CallSlot::Done { .. }) => SlotDecision::Conflict,
                Some(CallSlot::Evicted(prior)) if prior == request => SlotDecision::Evicted,
                Some(CallSlot::Evicted(_)) => SlotDecision::Conflict,
                Some(CallSlot::InFlight) => SlotDecision::Wait,
            };
            match decision {
                SlotDecision::Vacant => {
                    slots.insert(key, CallSlot::InFlight);
                    return Ok(CallAdmission::Proceed);
                }
                SlotDecision::Replay(response) => return Ok(CallAdmission::Replay(response)),
                SlotDecision::Conflict => return Ok(CallAdmission::Conflict),
                SlotDecision::Evicted => return Ok(CallAdmission::Evicted),
                SlotDecision::Wait => {
                    #[cfg(test)]
                    self.wait_probe.entered();
                    slots = self
                        .finished
                        .wait(slots)
                        .map_err(|_| GatewayError::StateUnavailable)?;
                }
            }
        }
    }

    fn complete(
        &self,
        key: (AttemptId, CallId),
        request: Request,
        response: Response,
    ) -> Result<(), GatewayError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        slots.insert(
            key,
            CallSlot::Done {
                request,
                response,
                sequence,
            },
        );
        let retained = slots
            .values()
            .filter(|slot| matches!(slot, CallSlot::Done { .. }))
            .count();
        if retained > MAX_RETAINED_RESULTS {
            // 逐出最早完成的结果；保留请求身份，让「结果已逐出」与「从未调用」可区分。
            let oldest = slots
                .iter()
                .filter_map(|(key, slot)| match slot {
                    CallSlot::Done { sequence, .. } => Some((*sequence, key.clone())),
                    _ => None,
                })
                .min_by_key(|(sequence, _)| *sequence)
                .map(|(_, key)| key);
            if let Some(key) = oldest
                && let Some(CallSlot::Done { request, .. }) = slots.remove(&key)
            {
                slots.insert(key, CallSlot::Evicted(request));
            }
        }
        drop(slots);
        self.finished.notify_all();
        Ok(())
    }

    fn abandon(&self, key: &(AttemptId, CallId)) -> Result<(), GatewayError> {
        self.slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?
            .remove(key);
        self.finished.notify_all();
        Ok(())
    }
}

/// 内存 Gateway 适配器，响应可配置且确定。
pub struct MemoryGateway {
    guard: Arc<dyn AttemptGuard>,
    audit: Arc<dyn AuditLedger>,
    clock: Arc<dyn Clock>,
    permissions: PermissionLayers,
    tool_responses: Mutex<BTreeMap<ToolId, Value>>,
    model_responses: Mutex<BTreeMap<(String, String), String>>,
    tool_calls: CallTable<ToolCall, Result<ToolResult, GatewayError>>,
    model_calls: CallTable<ModelRequest, Result<ModelResponse, GatewayError>>,
}

impl MemoryGateway {
    /// 由显式注入的运行时状态、审计、时钟与策略接缝创建适配器。
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
            tool_calls: CallTable::new(),
            model_calls: CallTable::new(),
        }
    }

    /// 为某个逻辑工具配置确定的结果。
    pub fn set_tool_response(&self, tool: ToolId, output: Value) -> Result<(), GatewayError> {
        self.lock(&self.tool_responses)?.insert(tool, output);
        Ok(())
    }

    /// 为某个供应商／模型组合配置确定的结果。
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

    /// 拒绝与冲突审计的独立事件 ID：`{base}:deny:{reason}`。
    /// 同一调用因不同原因被多次拒绝时各留一条记录，且永不与基础 ID 上的 Intent/Outcome 冲突。
    fn denial_audit_event_id(
        &self,
        base: &AuditEventId,
        error: &GatewayError,
    ) -> Result<AuditEventId, GatewayError> {
        AuditEventId::parse(format!("{base}:deny:{}", error.audit_reason()))
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
            .map_err(|error| match error {
                // 同一事件 ID 被不同内容复用：这是调用方的 call ID 冲突，不是账本不可用。
                AuditError::IdempotencyConflict { .. } => GatewayError::CallConflict,
                _ => GatewayError::AuditUnavailable,
            })
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

    /// 拒绝准入：为当前拒绝原因派生独立审计 ID 并追加 deny outcome；ID 派生失败按其错误返回。
    fn deny_admission(
        &self,
        error: GatewayError,
        audit_id: &AuditEventId,
        action: &str,
        capability: &JobCapability,
        tool: ToolId,
        call_id: &CallId,
    ) -> GatewayError {
        match self.denial_audit_event_id(audit_id, &error) {
            Ok(denial_id) => self.audit_denial(denial_id, action, capability, tool, call_id, error),
            Err(id_error) => id_error,
        }
    }

    /// 已占位的工具调用主体：Intent → 读取配置响应 → Outcome。只有账本已写入终局
    /// `Outcome` 的结局才是 `Recorded`，其余情况占位由调用方守卫归还。
    fn execute_tool(
        &self,
        audit_id: &AuditEventId,
        capability: &JobCapability,
        call: &ToolCall,
    ) -> Execution<ToolResult> {
        if let Err(error) = self.append_audit(
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
        ) {
            return Execution::Unrecorded(error);
        }

        let output = match self
            .lock(&self.tool_responses)
            .map(|responses| responses.get(call.tool()).cloned())
        {
            Ok(Some(output)) => output,
            Ok(None) => {
                let error = GatewayError::ToolUnavailable;
                return match self.append_audit(
                    audit_id.clone(),
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
                ) {
                    Ok(()) => Execution::Recorded(Err(error)),
                    Err(audit_error) => Execution::Unrecorded(audit_error),
                };
            }
            Err(error) => return Execution::Unrecorded(error),
        };
        let result = ToolResult {
            call_id: call.call_id().clone(),
            tool: call.tool().clone(),
            output,
        };
        match self.append_audit(
            audit_id.clone(),
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
        ) {
            Ok(()) => Execution::Recorded(Ok(result)),
            Err(error) => Execution::Unrecorded(error),
        }
    }

    /// 已占位的模型中继主体：Intent → 读取配置响应 → Outcome。结局语义同 `execute_tool`。
    fn execute_relay(
        &self,
        audit_id: &AuditEventId,
        capability: &JobCapability,
        relay_tool: &ToolId,
        request: &ModelRequest,
    ) -> Execution<ModelResponse> {
        if let Err(error) = self.append_audit(
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
        ) {
            return Execution::Unrecorded(error);
        }
        let text = match self.lock(&self.model_responses).map(|responses| {
            responses
                .get(&(request.provider.clone(), request.model.clone()))
                .cloned()
        }) {
            Ok(Some(text)) => text,
            Ok(None) => {
                let error = GatewayError::ModelUnavailable;
                return match self.append_audit(
                    audit_id.clone(),
                    AuditPhase::Outcome,
                    "gateway.model_relay",
                    capability,
                    relay_tool.clone(),
                    json!({
                        "call_id": request.call_id.as_str(),
                        "tool": "model.relay",
                        "provider": request.provider,
                        "model": request.model,
                        "fence": capability.fence().get(),
                        "decision": "error",
                        "reason": error.audit_reason(),
                    }),
                ) {
                    Ok(()) => Execution::Recorded(Err(error)),
                    Err(audit_error) => Execution::Unrecorded(audit_error),
                };
            }
            Err(error) => return Execution::Unrecorded(error),
        };
        let response = ModelResponse {
            call_id: request.call_id.clone(),
            provider: request.provider.clone(),
            model: request.model.clone(),
            text,
        };
        match self.append_audit(
            audit_id.clone(),
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
        ) {
            Ok(()) => Execution::Recorded(Ok(response)),
            Err(error) => Execution::Unrecorded(error),
        }
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
        let authorization = admission::Authorization {
            capability,
            tool: call.tool(),
            audit_id: &audit_id,
            call_id: call.call_id(),
            action: "gateway.tool_call",
        };
        authorization.check(self)?;

        match self
            .tool_calls
            .admit_checked(call_key.clone(), &call, || authorization.check(self))?
        {
            CallAdmission::Replay(result) => return result,
            CallAdmission::Proceed => {}
            CallAdmission::Conflict => {
                return Err(self.deny_admission(
                    GatewayError::CallConflict,
                    &audit_id,
                    "gateway.tool_call",
                    capability,
                    call.tool().clone(),
                    call.call_id(),
                ));
            }
            CallAdmission::Evicted => {
                return Err(self.deny_admission(
                    GatewayError::CallResultEvicted,
                    &audit_id,
                    "gateway.tool_call",
                    capability,
                    call.tool().clone(),
                    call.call_id(),
                ));
            }
        }

        let guard = SlotGuard::new(&self.tool_calls, call_key);
        match self.execute_tool(&audit_id, capability, &call) {
            Execution::Recorded(result) => {
                guard.complete(call, result.clone())?;
                result
            }
            Execution::Unrecorded(error) => {
                drop(guard);
                Err(error)
            }
        }
    }

    fn relay(
        &self,
        capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError> {
        let relay_tool = ToolId::from_static("model.relay");
        let call_key = (capability.attempt_id(), request.call_id.clone());
        let audit_id = self.audit_event_id("gateway.model.v1", capability, &request.call_id)?;
        let authorization = admission::Authorization {
            capability,
            tool: &relay_tool,
            audit_id: &audit_id,
            call_id: &request.call_id,
            action: "gateway.model_relay",
        };
        authorization.check(self)?;

        match self
            .model_calls
            .admit_checked(call_key.clone(), &request, || authorization.check(self))?
        {
            CallAdmission::Replay(response) => return response,
            CallAdmission::Proceed => {}
            CallAdmission::Conflict => {
                return Err(self.deny_admission(
                    GatewayError::CallConflict,
                    &audit_id,
                    "gateway.model_relay",
                    capability,
                    relay_tool,
                    &request.call_id,
                ));
            }
            CallAdmission::Evicted => {
                return Err(self.deny_admission(
                    GatewayError::CallResultEvicted,
                    &audit_id,
                    "gateway.model_relay",
                    capability,
                    relay_tool,
                    &request.call_id,
                ));
            }
        }

        let guard = SlotGuard::new(&self.model_calls, call_key);
        match self.execute_relay(&audit_id, capability, &relay_tool, &request) {
            Execution::Recorded(response) => {
                guard.complete(request, response.clone())?;
                response
            }
            Execution::Unrecorded(error) => {
                drop(guard);
                Err(error)
            }
        }
    }
}
