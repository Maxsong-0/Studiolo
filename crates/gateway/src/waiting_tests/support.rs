//! 两个真实 Gateway 调用与私有等待观测；不使用初检计数或 sleep 判断时序。

use super::audit::GatedAudit;
use crate::*;
use std::{
    sync::{Weak, mpsc},
    thread,
    time::Duration,
};

pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
pub(super) enum Branch {
    Invoke,
    Relay,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Admission {
    Replay,
    Proceed,
    Conflict,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum Revocation {
    Cancelled,
    Fence,
    Expired,
    Unavailable,
    Unknown,
}

pub(super) struct LiveGuard {
    state: Mutex<Result<AttemptSnapshot, AttemptGuardError>>,
    pub(super) gateway: Mutex<Weak<MemoryGateway>>,
}

impl AttemptGuard for LiveGuard {
    fn inspect(&self, _: TaskId, _: AttemptId) -> Result<AttemptSnapshot, AttemptGuardError> {
        assert_slots_unlocked(&self.gateway);
        *self.state.lock().unwrap()
    }
}

pub(super) fn assert_slots_unlocked(gateway: &Mutex<Weak<MemoryGateway>>) {
    let gateway = gateway.lock().unwrap().upgrade();
    if let Some(gateway) = gateway {
        assert_can_lock(&gateway.tool_calls.slots);
        assert_can_lock(&gateway.model_calls.slots);
    }
}

fn assert_can_lock<T>(slots: &Mutex<T>) {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        match slots.try_lock() {
            Ok(_) => return,
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("slot lock poisoned"),
            Err(std::sync::TryLockError::WouldBlock) => {
                // 允许其他线程短暂争用；当前回调若仍持锁，就无法在期限内重新取得它。
                assert!(
                    std::time::Instant::now() < deadline,
                    "guard/audit held slot lock"
                );
                thread::yield_now();
            }
        }
    }
}

pub(super) struct Fixture {
    pub(super) gateway: Arc<MemoryGateway>,
    pub(super) audit: Arc<GatedAudit>,
    guard: Arc<LiveGuard>,
    clock: Arc<ManualClock>,
    capability: Arc<JobCapability>,
    branch: Branch,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.audit.cancel_first();
    }
}

impl Fixture {
    pub(super) fn new(branch: Branch) -> Self {
        let fence = Fence::new(7).unwrap();
        let guard = Arc::new(LiveGuard {
            state: Mutex::new(Ok(AttemptSnapshot::active(fence))),
            gateway: Mutex::new(Weak::new()),
        });
        let audit = Arc::new(GatedAudit::new());
        let clock = Arc::new(ManualClock::new(1_000));
        let tools = PermissionSet::new([
            ToolId::from_static("web.fetch"),
            ToolId::from_static("model.relay"),
        ]);
        let gateway = Arc::new(MemoryGateway::new(
            guard.clone(),
            audit.clone(),
            clock.clone(),
            PermissionLayers::new(tools.clone(), tools.clone(), tools.clone()),
        ));
        gateway
            .set_tool_response(ToolId::from_static("web.fetch"), json!({"result": "first"}))
            .unwrap();
        gateway
            .set_model_response("local", "fixture", "first")
            .unwrap();
        *guard.gateway.lock().unwrap() = Arc::downgrade(&gateway);
        *audit.gateway.lock().unwrap() = Arc::downgrade(&gateway);
        Self {
            gateway,
            audit,
            guard,
            clock,
            branch,
            capability: Arc::new(JobCapability::new(
                TaskId::new(),
                AttemptId::new(),
                fence,
                tools,
                2_000,
            )),
        }
    }

    pub(super) fn call(&self, different: bool) -> Result<(), GatewayError> {
        call(&self.gateway, &self.capability, self.branch, different)
    }

    pub(super) fn spawn(&self, different: bool) -> thread::JoinHandle<Result<(), GatewayError>> {
        let gateway = self.gateway.clone();
        let capability = self.capability.clone();
        let branch = self.branch;
        thread::spawn(move || call(&gateway, &capability, branch, different))
    }

    pub(super) fn arm_waiter(&self) -> mpsc::Receiver<()> {
        match self.branch {
            Branch::Invoke => self.gateway.tool_calls.wait_probe.arm(),
            Branch::Relay => self.gateway.model_calls.wait_probe.arm(),
        }
    }

    pub(super) fn wait_entered(&self, receiver: mpsc::Receiver<()>) {
        receiver
            .recv_timeout(TIMEOUT)
            .expect("second call never observed InFlight");
        // 通知发自 Wait 分支持锁处；重新取得槽锁，证明该调用已把锁交给 Condvar。
        // 首个调用仍停在独立审计夹具中，因此此时不可能已经完成准入。
        match self.branch {
            Branch::Invoke => drop(self.gateway.tool_calls.slots.lock().unwrap()),
            Branch::Relay => drop(self.gateway.model_calls.slots.lock().unwrap()),
        }
    }

    pub(super) fn revoke(&self, reason: Revocation) -> GatewayError {
        match reason {
            Revocation::Cancelled => {
                *self.guard.state.lock().unwrap() =
                    Ok(AttemptSnapshot::cancelled(self.capability.fence()));
                GatewayError::Cancelled
            }
            Revocation::Fence => {
                *self.guard.state.lock().unwrap() =
                    Ok(AttemptSnapshot::active(Fence::new(8).unwrap()));
                GatewayError::FenceMismatch
            }
            Revocation::Expired => {
                self.clock.set(2_000);
                GatewayError::CapabilityExpired
            }
            Revocation::Unavailable => {
                *self.guard.state.lock().unwrap() = Err(AttemptGuardError::Unavailable);
                GatewayError::AttemptGuardUnavailable
            }
            Revocation::Unknown => {
                *self.guard.state.lock().unwrap() = Err(AttemptGuardError::UnknownAttempt);
                GatewayError::AttemptNotActive
            }
        }
    }

    pub(super) fn restore(&self) {
        *self.guard.state.lock().unwrap() = Ok(AttemptSnapshot::active(self.capability.fence()));
        self.clock.set(1_000);
    }

    pub(super) fn assert_slot(&self, admission: Admission) {
        let key = (
            self.capability.attempt_id(),
            CallId::parse("waiting-call").unwrap(),
        );
        let (exists, completed) = match self.branch {
            Branch::Invoke => {
                let slots = self.gateway.tool_calls.slots.lock().unwrap();
                (
                    slots.contains_key(&key),
                    matches!(slots.get(&key), Some(CallSlot::Done { .. })),
                )
            }
            Branch::Relay => {
                let slots = self.gateway.model_calls.slots.lock().unwrap();
                (
                    slots.contains_key(&key),
                    matches!(slots.get(&key), Some(CallSlot::Done { .. })),
                )
            }
        };
        match admission {
            Admission::Proceed => assert!(!exists, "rejected owner left its InFlight slot"),
            Admission::Replay | Admission::Conflict => {
                assert!(completed, "rejection removed another caller's result")
            }
        }
    }
}

fn call(
    gateway: &MemoryGateway,
    capability: &JobCapability,
    branch: Branch,
    different: bool,
) -> Result<(), GatewayError> {
    let id = CallId::parse("waiting-call").unwrap();
    match branch {
        Branch::Invoke => gateway
            .invoke(
                capability,
                ToolCall::new(
                    id,
                    ToolId::from_static("web.fetch"),
                    json!({"variant": different}),
                ),
            )
            .map(|_| ()),
        Branch::Relay => gateway
            .relay(
                capability,
                ModelRequest::new(
                    id,
                    "local",
                    "fixture",
                    if different { "different" } else { "original" },
                ),
            )
            .map(|_| ()),
    }
}
