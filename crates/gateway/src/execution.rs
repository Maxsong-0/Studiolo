//! 已占位调用的执行主体：终局 Outcome（含失败终态）落账即封口，未落账则归还占位以允许重试。

use crate::{
    GatewayError, JobCapability, MemoryGateway, ModelRequest, ModelResponse, ToolCall, ToolResult,
};
use serde_json::json;
use studiolo_audit_ledger::{AuditEventId, AuditPhase};
use studiolo_core::ToolId;

/// 一次已占位执行的结局：`Recorded` 表示账本已写入终局 `Outcome`（成功或失败），该
/// call ID 从此封口；`Unrecorded` 表示没有任何 `Outcome` 落账，占位应归还以允许重试。
pub(crate) enum Execution<Response> {
    Recorded(Result<Response, GatewayError>),
    Unrecorded(GatewayError),
}

impl MemoryGateway {
    /// 已占位的工具调用主体：Intent → 读取配置响应 → Outcome。只有账本已写入终局
    /// `Outcome` 的结局才是 `Recorded`，其余情况占位由调用方守卫归还。
    pub(crate) fn execute_tool(
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
    pub(crate) fn execute_relay(
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
