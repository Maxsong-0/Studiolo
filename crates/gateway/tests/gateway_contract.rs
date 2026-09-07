//! Public contract tests for the M0 in-memory Gateway adapter.

use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
    MemoryAuditLedger,
};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};
use studiolo_gateway::{
    AttemptGuard, AttemptGuardError, AttemptSnapshot, CallId, Gateway, GatewayError, JobCapability,
    MAX_RETAINED_RESULTS, ManualClock, MemoryAttemptGuard, MemoryGateway, ModelRequest,
    ModelResponse, PermissionLayers, ToolCall, ToolResult,
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

/// 拒绝与冲突审计使用的独立事件 ID（契约 §3）。
fn denial_audit_event_id(base: &AuditEventId, reason: &str) -> AuditEventId {
    AuditEventId::parse(format!("{base}:deny:{reason}")).unwrap()
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
        .query(&AuditFilter::default().with_event_id(denial_audit_event_id(
            &gateway_audit_event_id("gateway.tool.v1", &capability, &call_id),
            "tool_not_allowed",
        )))
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
        .query(&AuditFilter::default().with_event_id(denial_audit_event_id(
            &gateway_audit_event_id("gateway.tool.v1", &capability, &call_id),
            "fence_mismatch",
        )))
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
        .query(&AuditFilter::default().with_event_id(denial_audit_event_id(
            &gateway_audit_event_id("gateway.tool.v1", &capability, &call_id),
            "capability_expired",
        )))
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
    let conflict_id = denial_audit_event_id(&base_id, "call_conflict");
    let conflicts = audit
        .query(&AuditFilter::default().with_event_id(conflict_id))
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        conflicts[0].event().payload().as_value()["reason"],
        "call_conflict"
    );
}

#[test]
fn repeated_denials_of_one_call_each_keep_their_own_audit_record_and_reason() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(21).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard.clone(),
        audit.clone(),
        Arc::new(ManualClock::new(2_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    gateway
        .set_tool_response(tool.clone(), json!({"should_not": "escape"}))
        .unwrap();
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("denied-twice").unwrap();
    let call = ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"}));

    // 从未完成过的调用先因过期被拒，再因取消被拒：第二次必须返回真实原因并留下自己的记录。
    assert_eq!(
        gateway.invoke(&capability, call.clone()),
        Err(GatewayError::CapabilityExpired)
    );
    guard
        .set(task, attempt, AttemptSnapshot::cancelled(fence))
        .unwrap();
    assert_eq!(
        gateway.invoke(&capability, call),
        Err(GatewayError::Cancelled)
    );

    let base_id = gateway_audit_event_id("gateway.tool.v1", &capability, &call_id);
    assert!(
        audit
            .query(&AuditFilter::default().with_event_id(base_id.clone()))
            .unwrap()
            .is_empty()
    );
    for reason in ["capability_expired", "cancelled"] {
        let records = audit
            .query(&AuditFilter::default().with_event_id(denial_audit_event_id(&base_id, reason)))
            .unwrap();
        assert_eq!(records.len(), 1, "{reason}");
        assert_eq!(records[0].event().phase(), AuditPhase::Outcome);
        assert_eq!(records[0].event().payload().as_value()["reason"], reason);
    }
}

/// 第一次 append 进入后阻塞，直到测试放行；用于把并发调用钉在「首次调用正在执行」的时刻。
struct GatedAuditLedger {
    inner: MemoryAuditLedger,
    appends: AtomicUsize,
    entered: (Mutex<bool>, Condvar),
    released: (Mutex<bool>, Condvar),
}

impl GatedAuditLedger {
    fn new() -> Self {
        Self {
            inner: MemoryAuditLedger::new(),
            appends: AtomicUsize::new(0),
            entered: (Mutex::new(false), Condvar::new()),
            released: (Mutex::new(false), Condvar::new()),
        }
    }
}

impl AuditLedger for GatedAuditLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if self.appends.fetch_add(1, Ordering::SeqCst) == 0 {
            raise_flag(&self.entered);
            wait_flag(&self.released, "release of the first append");
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

/// 计数守卫：第二次 inspect 说明第二个调用已通过授权、即将进入幂等槽判定。
struct CountingGuard {
    inner: MemoryAttemptGuard,
    inspections: AtomicUsize,
    second_inspection: (Mutex<bool>, Condvar),
}

impl AttemptGuard for CountingGuard {
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError> {
        if self.inspections.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
            raise_flag(&self.second_inspection);
        }
        self.inner.inspect(task_id, attempt_id)
    }
}

fn raise_flag((flag, condvar): &(Mutex<bool>, Condvar)) {
    *flag.lock().unwrap() = true;
    condvar.notify_all();
}

fn wait_flag((flag, condvar): &(Mutex<bool>, Condvar), what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut raised = flag.lock().unwrap();
    while !*raised {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("timed out waiting for {what}"));
        raised = condvar.wait_timeout(raised, remaining).unwrap().0;
    }
}

#[test]
fn concurrent_identical_calls_execute_once_and_share_the_first_result() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(22).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(CountingGuard {
        inner: MemoryAttemptGuard::new(),
        inspections: AtomicUsize::new(0),
        second_inspection: (Mutex::new(false), Condvar::new()),
    });
    guard
        .inner
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(GatedAuditLedger::new());
    let gateway = Arc::new(MemoryGateway::new(
        guard.clone(),
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    ));
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let capability = Arc::new(JobCapability::new(task, attempt, fence, granted, 2_000));
    let call_id = CallId::parse("concurrent-call").unwrap();
    let call = ToolCall::new(call_id.clone(), tool, json!({"path": "/pricing"}));

    let spawn = |gateway: Arc<MemoryGateway>, capability: Arc<JobCapability>, call: ToolCall| {
        thread::spawn(move || gateway.invoke(&capability, call))
    };
    let first = spawn(gateway.clone(), capability.clone(), call.clone());
    // 首次调用已经占位并停在 Intent 追加里；此时第二个同键调用进入。
    wait_flag(&audit.entered, "the first intent append");
    let second = spawn(gateway.clone(), capability.clone(), call);
    wait_flag(&guard.second_inspection, "the second authorization");
    raise_flag(&audit.released);

    let first = first.join().unwrap().unwrap();
    let second = second.join().unwrap().unwrap();
    assert_eq!(first, second);
    // 整个过程只有一次 Intent 与一次 Outcome：第二个调用等待后复用结果，没有再执行。
    assert_eq!(audit.appends.load(Ordering::SeqCst), 2);
    let base_id = gateway_audit_event_id("gateway.tool.v1", &capability, &call_id);
    let records = audit
        .query(&AuditFilter::default().with_event_id(base_id))
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
}

#[test]
fn a_recorded_failure_seals_its_call_id_and_replays_without_executing() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(23).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let relay_tool = ToolId::from_static("model.relay");
    let granted = PermissionSet::new([tool.clone(), relay_tool]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let audit = Arc::new(MemoryAuditLedger::new());
    let gateway = MemoryGateway::new(
        guard.clone(),
        audit.clone(),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    );
    let capability = JobCapability::new(task, attempt, fence, granted, 2_000);
    let call_id = CallId::parse("sealed-failure").unwrap();
    let call = ToolCall::new(call_id.clone(), tool.clone(), json!({"path": "/late"}));
    let request = ModelRequest::new(call_id.clone(), "local", "late", "summarize");

    assert_eq!(
        gateway.invoke(&capability, call.clone()),
        Err(GatewayError::ToolUnavailable)
    );
    assert_eq!(
        gateway.relay(&capability, request.clone()),
        Err(GatewayError::ModelUnavailable)
    );
    // 运维随后补上配置：同 call ID 的重试不再执行，账本里的失败终态被原样重放。
    gateway
        .set_tool_response(tool.clone(), json!({"status": "late"}))
        .unwrap();
    gateway.set_model_response("local", "late", "text").unwrap();
    assert_eq!(
        gateway.invoke(&capability, call.clone()),
        Err(GatewayError::ToolUnavailable)
    );
    assert_eq!(
        gateway.relay(&capability, request.clone()),
        Err(GatewayError::ModelUnavailable)
    );
    for domain in ["gateway.tool.v1", "gateway.model.v1"] {
        let records = audit
            .query(
                &AuditFilter::default().with_event_id(gateway_audit_event_id(
                    domain,
                    &capability,
                    &call_id,
                )),
            )
            .unwrap();
        assert_eq!(
            records.len(),
            2,
            "{domain}: replay must not append a new Intent/Outcome"
        );
    }
    // 新的 call ID 才是一次新的逻辑调用。
    let fresh = CallId::parse("sealed-failure-2").unwrap();
    assert!(
        gateway
            .invoke(
                &capability,
                ToolCall::new(fresh.clone(), tool, json!({"path": "/late"}))
            )
            .is_ok()
    );
    assert!(
        gateway
            .relay(
                &capability,
                ModelRequest::new(fresh, "local", "late", "summarize")
            )
            .is_ok()
    );
    // 失败终态的重放同样先重验授权。
    guard
        .set(task, attempt, AttemptSnapshot::cancelled(fence))
        .unwrap();
    assert_eq!(
        gateway.invoke(&capability, call),
        Err(GatewayError::Cancelled)
    );
    assert_eq!(
        gateway.relay(&capability, request),
        Err(GatewayError::Cancelled)
    );
}

struct PanickingLedger {
    inner: MemoryAuditLedger,
    panic_next_intent: AtomicBool,
}

impl AuditLedger for PanickingLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Intent
            && self.panic_next_intent.swap(false, Ordering::SeqCst)
        {
            panic!("injected adapter panic during Intent append");
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

#[test]
fn a_panicking_adapter_releases_its_in_flight_slot_for_the_next_caller() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(24).unwrap();
    let tool = ToolId::from_static("web.fetch");
    let granted = PermissionSet::new([tool.clone()]);
    let guard = Arc::new(MemoryAttemptGuard::new());
    guard
        .set(task, attempt, AttemptSnapshot::active(fence))
        .unwrap();
    let gateway = Arc::new(MemoryGateway::new(
        guard,
        Arc::new(PanickingLedger {
            inner: MemoryAuditLedger::new(),
            panic_next_intent: AtomicBool::new(true),
        }),
        Arc::new(ManualClock::new(1_000)),
        PermissionLayers::new(granted.clone(), granted.clone(), granted.clone()),
    ));
    gateway
        .set_tool_response(tool.clone(), json!({"status": "ok"}))
        .unwrap();
    let capability = Arc::new(JobCapability::new(task, attempt, fence, granted, 2_000));
    let call = ToolCall::new(
        CallId::parse("panicking-call").unwrap(),
        tool,
        json!({"path": "/pricing"}),
    );

    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gateway.invoke(&capability, call.clone())
    }));
    assert!(
        first.is_err(),
        "the injected panic must unwind out of invoke"
    );

    // 同键的下一次调用必须能取得占位并执行，而不是在 Condvar 上永久等待。
    let (sender, receiver) = mpsc::channel();
    let worker = {
        let gateway = gateway.clone();
        let capability = capability.clone();
        let call = call.clone();
        thread::spawn(move || {
            let _ = sender.send(gateway.invoke(&capability, call));
        })
    };
    let second = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("second call hung on the leaked in-flight slot");
    worker.join().unwrap();
    assert_eq!(second.unwrap().output(), &json!({"status": "ok"}));
}

#[test]
fn retained_results_are_bounded_and_evicted_replays_are_rejected_with_audit() {
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let fence = Fence::new(25).unwrap();
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
    let call = |index: usize| {
        ToolCall::new(
            CallId::parse(format!("bounded-{index}")).unwrap(),
            tool.clone(),
            json!({"index": index}),
        )
    };

    for index in 0..=MAX_RETAINED_RESULTS {
        gateway.invoke(&capability, call(index)).unwrap();
    }
    // 最早完成的结果已逐出：同请求重放得到独立审计的结构化拒绝，且不重新执行。
    assert_eq!(
        gateway.invoke(&capability, call(0)),
        Err(GatewayError::CallResultEvicted)
    );
    let first_id = CallId::parse("bounded-0").unwrap();
    let base_id = gateway_audit_event_id("gateway.tool.v1", &capability, &first_id);
    assert_eq!(
        audit
            .query(&AuditFilter::default().with_event_id(base_id.clone()))
            .unwrap()
            .len(),
        2
    );
    let evicted = audit
        .query(
            &AuditFilter::default()
                .with_event_id(denial_audit_event_id(&base_id, "call_result_evicted")),
        )
        .unwrap();
    assert_eq!(evicted.len(), 1);
    assert_eq!(
        evicted[0].event().payload().as_value()["reason"],
        "call_result_evicted"
    );
    // 逐出槽仍保留请求身份：异请求是内容冲突而不是首次调用。
    assert_eq!(
        gateway.invoke(
            &capability,
            ToolCall::new(first_id, tool.clone(), json!({"index": "changed"})),
        ),
        Err(GatewayError::CallConflict)
    );
    // 最近完成的结果仍可重放。
    assert_eq!(
        gateway
            .invoke(&capability, call(MAX_RETAINED_RESULTS))
            .unwrap()
            .output(),
        &json!({"status": "ok"})
    );
}
