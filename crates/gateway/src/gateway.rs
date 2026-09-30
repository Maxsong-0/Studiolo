//! Gateway 接缝与内存适配器主体：依赖注入、实时授权检查，以及 invoke／relay 的调用编排。

use crate::{
    AttemptGuard, AttemptGuardError, AttemptSnapshot, Clock, GatewayError, JobCapability,
    ModelRequest, ModelResponse, PermissionLayers, ToolCall, ToolResult, admission,
    call_table::{CallAdmission, CallTable, RequestIdentity},
    execution::Execution,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};
use studiolo_audit_ledger::AuditLedger;
use studiolo_core::ToolId;

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

/// 内存 Gateway 适配器，响应可配置且确定。
pub struct MemoryGateway {
    guard: Arc<dyn AttemptGuard>,
    pub(crate) audit: Arc<dyn AuditLedger>,
    clock: Arc<dyn Clock>,
    permissions: PermissionLayers,
    pub(crate) tool_responses: Mutex<BTreeMap<ToolId, Value>>,
    pub(crate) model_responses: Mutex<BTreeMap<(String, String), String>>,
    pub(crate) tool_calls: CallTable<Result<ToolResult, GatewayError>>,
    pub(crate) model_calls: CallTable<Result<ModelResponse, GatewayError>>,
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

    pub(crate) fn lock<'a, T>(
        &self,
        mutex: &'a Mutex<T>,
    ) -> Result<MutexGuard<'a, T>, GatewayError> {
        mutex.lock().map_err(|_| GatewayError::StateUnavailable)
    }

    fn inspect(&self, capability: &JobCapability) -> Result<AttemptSnapshot, GatewayError> {
        self.guard
            .inspect(capability.task_id(), capability.attempt_id())
            .map_err(|error| match error {
                AttemptGuardError::UnknownAttempt => GatewayError::AttemptNotActive,
                AttemptGuardError::Unavailable => GatewayError::AttemptGuardUnavailable,
            })
    }

    pub(crate) fn authorize(
        &self,
        capability: &JobCapability,
        tool: &ToolId,
    ) -> Result<(), GatewayError> {
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

        let guard =
            match self
                .tool_calls
                .admit_checked(call_key, RequestIdentity::tool(&call), || {
                    authorization.check(self)
                })? {
                CallAdmission::Replay(result) => return result,
                CallAdmission::Proceed(guard) => guard,
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
            };
        match self.execute_tool(&audit_id, capability, &call) {
            Execution::Recorded(result) => {
                guard.complete(result.clone())?;
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

        let guard = match self.model_calls.admit_checked(
            call_key,
            RequestIdentity::model(&request),
            || authorization.check(self),
        )? {
            CallAdmission::Replay(response) => return response,
            CallAdmission::Proceed(guard) => guard,
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
        };
        match self.execute_relay(&audit_id, capability, &relay_tool, &request) {
            Execution::Recorded(response) => {
                guard.complete(response.clone())?;
                response
            }
            Execution::Unrecorded(error) => {
                drop(guard);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "admission_guard_tests.rs"]
mod admission_guard_tests;
