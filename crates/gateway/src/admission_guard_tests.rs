//! 第二次授权及拒绝审计 unwind 回归；先断言槽为空再同步重试，不创建可能挂死的线程。

use super::*;
use crate::{Gateway, ManualClock};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId};

#[derive(Clone, Copy)]
enum Branch {
    Invoke,
    Relay,
}

struct UnwindingGuard {
    calls: AtomicUsize,
    fence: Fence,
    panic_in_inspect: bool,
}

impl AttemptGuard for UnwindingGuard {
    fn inspect(&self, _: TaskId, _: AttemptId) -> Result<AttemptSnapshot, AttemptGuardError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            assert!(
                !self.panic_in_inspect,
                "injected panic in second authorization"
            );
            return Err(AttemptGuardError::Unavailable);
        }
        Ok(AttemptSnapshot::active(self.fence))
    }
}

struct UnwindingAudit {
    inner: MemoryAuditLedger,
    panic_on_denial: AtomicBool,
}

impl AuditLedger for UnwindingAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.payload().as_value()["decision"] == "deny"
            && self.panic_on_denial.swap(false, Ordering::SeqCst)
        {
            panic!("injected panic in second authorization denial audit");
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

fn exercise_unwind(branch: Branch, panic_in_inspect: bool) {
    let fence = Fence::new(7).unwrap();
    let tools = PermissionSet::new([
        ToolId::from_static("web.fetch"),
        ToolId::from_static("model.relay"),
    ]);
    let audit = Arc::new(UnwindingAudit {
        inner: MemoryAuditLedger::new(),
        panic_on_denial: AtomicBool::new(!panic_in_inspect),
    });
    let gateway = MemoryGateway::new(
        Arc::new(UnwindingGuard {
            calls: AtomicUsize::new(0),
            fence,
            panic_in_inspect,
        }),
        audit.clone(),
        Arc::new(ManualClock::new(100)),
        PermissionLayers::new(tools.clone(), tools.clone(), tools.clone()),
    );
    gateway
        .set_tool_response(ToolId::from_static("web.fetch"), serde_json::json!("ok"))
        .unwrap();
    gateway
        .set_model_response("local", "fixture", "ok")
        .unwrap();
    let capability = JobCapability::new(TaskId::new(), AttemptId::new(), fence, tools, 200);
    let id = crate::CallId::parse("unwind").unwrap();
    let key = (capability.attempt_id(), id.clone());
    let call = || match branch {
        Branch::Invoke => gateway
            .invoke(
                &capability,
                ToolCall::new(
                    id.clone(),
                    ToolId::from_static("web.fetch"),
                    serde_json::json!({"path": "/"}),
                ),
            )
            .map(|_| ()),
        Branch::Relay => gateway
            .relay(
                &capability,
                ModelRequest::new(id.clone(), "local", "fixture", "prompt"),
            )
            .map(|_| ()),
    };
    assert!(catch_unwind(AssertUnwindSafe(&call)).is_err());
    let vacant = match branch {
        Branch::Invoke => !gateway
            .tool_calls
            .state
            .try_lock()
            .unwrap()
            .slots
            .contains_key(&key),
        Branch::Relay => !gateway
            .model_calls
            .state
            .try_lock()
            .unwrap()
            .slots
            .contains_key(&key),
    };
    assert!(
        vacant,
        "second authorization panic leaked its in-flight slot"
    );
    // 已在当前唯一执行线程证明没有 InFlight，所以这个重试不可能等待遗留占位。
    call().unwrap();
    call().unwrap();
    let records = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 2, "only the successful retry executes");
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
}

#[test]
fn invoke_second_guard_panic_releases_slot() {
    exercise_unwind(Branch::Invoke, true);
}

#[test]
fn relay_second_guard_panic_releases_slot() {
    exercise_unwind(Branch::Relay, true);
}

#[test]
fn invoke_second_denial_audit_panic_releases_slot() {
    exercise_unwind(Branch::Invoke, false);
}

#[test]
fn relay_second_denial_audit_panic_releases_slot() {
    exercise_unwind(Branch::Relay, false);
}

#[test]
fn replay_authorization_panic_preserves_existing_result() {
    let table = CallTable::<()>::new();
    let key = (AttemptId::new(), crate::CallId::parse("completed").unwrap());
    let identity = RequestIdentity::model(&ModelRequest::new(
        key.1.clone(),
        "local",
        "fixture",
        "prompt",
    ));
    let CallAdmission::Proceed(guard) = table.admit(key.clone(), identity).unwrap() else {
        panic!("first request was not admitted");
    };
    guard.complete(()).unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| {
        table.admit_checked(key.clone(), identity, || {
            panic!("replay authorization panic")
        })
    }));
    assert!(result.is_err());
    assert!(matches!(
        table.admit(key, identity).unwrap(),
        CallAdmission::Replay(())
    ));
}
