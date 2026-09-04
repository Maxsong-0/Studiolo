//! Public contract tests for the M0 in-memory Gateway adapter.

use std::sync::Arc;

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger,
};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};
use studiolo_gateway::{
    AttemptSnapshot, CallId, Gateway, GatewayError, JobCapability, ManualClock, MemoryAttemptGuard,
    MemoryGateway, ModelRequest, ModelResponse, PermissionLayers, ToolCall, ToolResult,
};

struct UnavailableAuditLedger;

struct EchoGateway;

impl Gateway for EchoGateway {
    fn invoke(
        &self,
        _capability: &JobCapability,
        call: ToolCall,
    ) -> Result<ToolResult, GatewayError> {
        Ok(ToolResult::new(
            call.call_id().clone(),
            call.tool().clone(),
            call.arguments().clone(),
        ))
    }

    fn relay(
        &self,
        _capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError> {
        Ok(ModelResponse::new(
            request.call_id().clone(),
            request.provider(),
            request.model(),
            request.prompt(),
        ))
    }
}

impl AuditLedger for UnavailableAuditLedger {
    fn append(&self, _event: AuditEvent) -> Result<AuditRecord, AuditError> {
        Err(AuditError::Unavailable)
    }

    fn query(&self, _filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Err(AuditError::Unavailable)
    }
}

fn gateway_audit_event_id(
    domain: &str,
    capability: &JobCapability,
    call_id: &CallId,
) -> AuditEventId {
    AuditEventId::parse(format!(
        "{domain}:task={}:attempt={}:fence={}:call={}:{}",
        capability.task_id(),
        capability.attempt_id(),
        capability.fence().get(),
        call_id.as_str().len(),
        call_id,
    ))
    .unwrap()
}

#[test]
fn gateway_is_object_safe_and_allowed_tool_call_is_audited() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(7).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let seam: &dyn Gateway = &gateway;
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("invoke-001").unwrap();

    let result = seam
        .invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"})),
        )
        .unwrap();

    assert_eq!(result.output(), &json!({"status": "ok"}));
    let audit_event_id = gateway_audit_event_id("gateway.tool.v1", &capability, &call_id);
    let records = audit
        .query(&AuditFilter::default().with_event_id(audit_event_id))
        .unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.event().phase())
            .collect::<Vec<_>>(),
        vec![AuditPhase::Intent, AuditPhase::Outcome]
    );
}

#[test]
fn every_permission_layer_must_allow_the_tool_and_denial_is_audited() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(8).unwrap();
    let tool = ToolId::from_static("db.query");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), PermissionSet::default(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"rows": []}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("invoke-denied").unwrap();

    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"view": "orders"})),
        ),
        Err(GatewayError::ToolNotAllowed)
    );
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.tool.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[0].event().payload().as_value()["reason"],
        "tool_not_allowed"
    );
}

#[test]
fn model_relay_uses_the_same_capability_and_audit_boundary() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(9).unwrap();
    let relay_tool = ToolId::from_static("model.relay");
    let granted = PermissionSet::new([relay_tool]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_model_response("local", "fixture", "model result")
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("relay-001").unwrap();

    let response = gateway
        .relay(
            &capability,
            ModelRequest::new(call_id.clone(), "local", "fixture", "summarize"),
        )
        .unwrap();

    assert_eq!(response.text(), "model result");
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.model.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
}

#[test]
fn a_completed_call_is_still_rechecked_after_the_attempt_is_cancelled() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(10).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let gateway = MemoryGateway::new(
        guard.clone(),
        Arc::new(MemoryAuditLedger::new()),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call = ToolCall::new(
        CallId::parse("invoke-then-cancel").unwrap(),
        tool,
        json!({"path": "/pricing"}),
    );
    gateway.invoke(&capability, call.clone()).unwrap();
    guard
        .set(task, attempt, AttemptSnapshot::cancelled(fence))
        .unwrap();

    assert_eq!(
        gateway.invoke(&capability, call),
        Err(GatewayError::Cancelled)
    );
}

#[test]
fn an_unavailable_tool_resolves_its_persisted_intent_with_an_outcome() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(11).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("invoke-no-adapter").unwrap();

    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"})),
        ),
        Err(GatewayError::ToolUnavailable)
    );
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.tool.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[1].event().payload().as_value()["reason"],
        "tool_unavailable"
    );
}

#[test]
fn a_stale_fence_is_rejected_before_a_tool_result_can_be_released() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let stale_fence = Fence::new(11).unwrap();
    let live_fence = Fence::new(12).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(live_fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"should_not": "escape"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, stale_fence, granted, 2_000);
    let call_id = CallId::parse("invoke-stale-fence").unwrap();

    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"})),
        ),
        Err(GatewayError::FenceMismatch)
    );
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.tool.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[0].event().payload().as_value()["reason"],
        "fence_mismatch"
    );
}

#[test]
fn a_capability_expires_at_its_declared_deadline() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(13).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(2_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"should_not": "escape"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("invoke-expired").unwrap();

    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"})),
        ),
        Err(GatewayError::CapabilityExpired)
    );
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.tool.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].event().payload().as_value()["reason"],
        "capability_expired"
    );
}

#[test]
fn an_audit_intent_failure_fails_a_sensitive_call_closed() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(14).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let gateway = MemoryGateway::new(
        guard,
        Arc::new(UnavailableAuditLedger),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"should_not": "escape"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);

    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(
                CallId::parse("invoke-no-audit").unwrap(),
                tool,
                json!({"path": "/pricing"}),
            ),
        ),
        Err(GatewayError::AuditUnavailable)
    );
}

#[test]
fn an_unavailable_model_resolves_its_persisted_intent_with_an_outcome() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(15).unwrap();
    let relay_tool = ToolId::from_static("model.relay");
    let granted = PermissionSet::new([relay_tool]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("relay-no-adapter").unwrap();

    assert_eq!(
        gateway.relay(
            &capability,
            ModelRequest::new(call_id.clone(), "local", "missing", "summarize"),
        ),
        Err(GatewayError::ModelUnavailable)
    );
    let records = audit
        .query(
            &AuditFilter::default().with_event_id(gateway_audit_event_id(
                "gateway.model.v1",
                &capability,
                &call_id,
            )),
        )
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[1].event().payload().as_value()["reason"],
        "model_unavailable"
    );
}

#[test]
fn call_idempotency_is_scoped_to_an_attempt() {
    let task = TaskId::new();
    let first_attempt = AttemptId::new();
    let second_attempt = AttemptId::new();
    let first_fence = Fence::new(16).unwrap();
    let second_fence = Fence::new(17).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, first_attempt, AttemptSnapshot::active(first_fence))
        .unwrap();
    guard
        .set(task, second_attempt, AttemptSnapshot::active(second_fence))
        .unwrap();
    let gateway = MemoryGateway::new(
        guard,
        Arc::new(MemoryAuditLedger::new()),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let call_id = CallId::parse("runner-local-call-1").unwrap();

    gateway
        .invoke(
            &JobCapability::new(task, first_attempt, first_fence, granted.clone(), 2_000),
            ToolCall::new(call_id.clone(), tool.clone(), json!({"path": "/one"})),
        )
        .unwrap();
    let second = gateway.invoke(
        &JobCapability::new(task, second_attempt, second_fence, granted, 2_000),
        ToolCall::new(call_id, tool, json!({"path": "/two"})),
    );

    assert!(second.is_ok());
}

#[test]
fn gateway_dtos_are_usable_by_adapter_implementors() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(18).unwrap();
    let tool = ToolId::from_static("echo");
    let granted = PermissionSet::new([tool.clone(), ToolId::from_static("model.relay")]);
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let seam: &dyn Gateway = &EchoGateway;

    let tool_result = seam
        .invoke(
            &capability,
            ToolCall::new(
                CallId::parse("external-tool-adapter").unwrap(),
                tool,
                json!({"message": "hello"}),
            ),
        )
        .unwrap();
    assert_eq!(tool_result.output(), &json!({"message": "hello"}));

    let model_response = seam
        .relay(
            &capability,
            ModelRequest::new(
                CallId::parse("external-model-adapter").unwrap(),
                "local",
                "echo",
                "hello",
            ),
        )
        .unwrap();
    assert_eq!(model_response.text(), "hello");
}

#[test]
fn deserialization_cannot_bypass_call_id_validation() {
    assert!(serde_json::from_str::<CallId>("\"\"").is_err());
    assert!(serde_json::from_str::<CallId>("\"bad\\nidentifier\"").is_err());
    assert_eq!(
        serde_json::from_str::<CallId>("\"valid-call\"")
            .unwrap()
            .as_str(),
        "valid-call"
    );
}

#[test]
fn missing_permission_configuration_defaults_to_deny() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(19).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let gateway = MemoryGateway::new(
        guard,
        Arc::new(MemoryAuditLedger::new()),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::default(),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"should_not": "escape"}))
        .unwrap();

    assert_eq!(
        gateway.invoke(
            &JobCapability::new(task, attempt, fence, granted, 2_000),
            ToolCall::new(CallId::parse("default-deny").unwrap(), tool, json!({}),),
        ),
        Err(GatewayError::ToolNotAllowed)
    );
}

#[test]
fn identical_call_replays_the_result_but_changed_content_conflicts() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(20).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard,
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("idempotent-call").unwrap();
    let call = ToolCall::new(call_id.clone(), tool.clone(), json!({"path": "/one"}));

    let first = gateway.invoke(&capability, call.clone()).unwrap();
    assert_eq!(gateway.invoke(&capability, call).unwrap(), first);
    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(call_id.clone(), tool, json!({"path": "/two"})),
        ),
        Err(GatewayError::CallConflict)
    );

    let base_id = gateway_audit_event_id("gateway.tool.v1", &capability, &call_id);
    assert_eq!(
        audit
            .query(&AuditFilter::default().with_event_id(base_id.clone()))
            .unwrap()
            .len(),
        2
    );
    let conflict_id = AuditEventId::parse(format!("{base_id}:recheck:call_conflict")).unwrap();
    let conflicts = audit
        .query(&AuditFilter::default().with_event_id(conflict_id))
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        conflicts[0].event().payload().as_value()["reason"],
        "call_conflict"
    );
}
