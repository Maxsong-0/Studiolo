//! 真实调用的执行次序；不联网，也不需要供应商凭据。
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};
use studiolo_gateway::{
    AttemptSnapshot, CallId, DurableRelay, JobCapability, ManualClock, MemoryAttemptGuard,
    ModelEgress, PermissionLayers, RelayError, RelayGrant, RelayLimits, RelayPolicyGuard,
    RelayProvider, RelayPublisher, RelayServices, RelaySource, RelayTransport,
};

struct Policy(AtomicBool);
impl RelayPolicyGuard for Policy {
    fn inspect(&self, _: &RelayGrant, now: i64) -> Result<(), RelayError> {
        if self.0.load(Ordering::SeqCst) && now < 500 {
            Ok(())
        } else {
            Err(RelayError::Denied)
        }
    }
}
struct Publisher(AtomicBool);
impl RelayPublisher for Publisher {
    fn seal(&self, _: &RelayGrant, _: &CallId, raw: &Path, _: &str) -> Result<(), RelayError> {
        if self.0.load(Ordering::SeqCst) && raw.is_file() {
            Ok(())
        } else {
            Err(RelayError::Publication)
        }
    }
}
struct Transport {
    calls: AtomicUsize,
    fail: AtomicBool,
}
impl RelayTransport for Transport {
    fn execute(
        &self,
        _: &RelayGrant,
        _: &[u8],
        check: &dyn Fn() -> Result<(), RelayError>,
    ) -> Result<Vec<u8>, RelayError> {
        check()?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err(RelayError::Upstream)
        } else {
            Ok(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec())
        }
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    relay: DurableRelay,
    grant: RelayGrant,
    guard: Arc<MemoryAttemptGuard>,
    clock: Arc<ManualClock>,
    policy: Arc<Policy>,
    publisher: Arc<Publisher>,
    transport: Arc<Transport>,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("gateway-raw");
        std::fs::create_dir(&raw).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // `tempfile` 建出的目录权限随平台与 `umask` 而定；生产存储刻意拒绝
            // 带有任何 `group`/`other` 权限的数据库父目录。
            for path in [directory.path(), raw.as_path()] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let allowed = PermissionSet::new([ToolId::from_static("model.relay")]);
        let task = TaskId::new();
        let attempt = AttemptId::new();
        let fence = Fence::new(1).unwrap();
        let guard = Arc::new(MemoryAttemptGuard::new());
        guard
            .set(task, attempt, AttemptSnapshot::active(fence))
            .unwrap();
        let clock = Arc::new(ManualClock::new(100));
        let policy = Arc::new(Policy(AtomicBool::new(true)));
        let publisher = Arc::new(Publisher(AtomicBool::new(true)));
        let transport = Arc::new(Transport {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        });
        let grant = RelayGrant {
            capability: JobCapability::new(task, attempt, fence, allowed.clone(), 1000),
            audience: "studiolo.model-relay.v1".into(),
            snapshot_id: "snapshot-1".into(),
            policy_hash: "policy-1".into(),
            role: "writer".into(),
            adapter: "claude".into(),
            provider: RelayProvider::Anthropic,
            model: "pinned-model".into(),
            sources: vec![RelaySource {
                source: "Studiolo/note.md".into(),
                revision: "r1".into(),
                sha256: "0".repeat(64),
                retain_until_unix_seconds: 1000,
                model_egress: ModelEgress::Providers(vec![RelayProvider::Anthropic]),
            }],
            limits: RelayLimits {
                request_bytes: 1024,
                response_bytes: 4096,
                response_tokens: 128,
                calls: 2,
                pacing_tools: 24,
                wrapup_tools: 120,
                max_tools: 160,
                budget_microusd: 2400,
                reservation_microusd: 1200,
                input_token_microusd: 1,
                output_token_microusd: 1,
                timeout_seconds: 30,
            },
        };
        let relay = DurableRelay::open(
            &directory.path().join("relay.db"),
            raw,
            RelayServices {
                guard: guard.clone(),
                policy: policy.clone(),
                clock: clock.clone(),
                audit: Arc::new(MemoryAuditLedger::new()),
                permissions: PermissionLayers::new(allowed.clone(), allowed.clone(), allowed),
                transport: transport.clone(),
                publisher: publisher.clone(),
            },
        )
        .unwrap();
        assert!(matches!(
            relay.issue(grant.clone()),
            Err(RelayError::Frozen)
        ));
        relay.set_frozen(false).unwrap();
        Self {
            directory,
            relay,
            grant,
            guard,
            clock,
            policy,
            publisher,
            transport,
        }
    }
    fn call(&self, token: &str, id: &str) -> Result<studiolo_gateway::RelayResponse, RelayError> {
        self.relay.relay(
            token,
            CallId::parse(id).unwrap(),
            "/v1/messages",
            br#"{"model":"pinned-model","messages":[]}"#,
        )
    }
}
#[test]
fn deny_and_local_only_cannot_issue_cloud_tickets() {
    for label in [ModelEgress::Deny, ModelEgress::LocalOnly] {
        let mut f = Fixture::new();
        f.grant.sources[0].model_egress = label;
        assert!(matches!(f.relay.issue(f.grant), Err(RelayError::Egress)));
        assert_eq!(f.transport.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn durable_result_replays_without_network_but_fence_and_policy_are_fresh() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    let first = f.call(ticket.expose(), "a").unwrap();
    assert_eq!(f.call(ticket.expose(), "a").unwrap(), first);
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 1);
    f.policy.0.store(false, Ordering::SeqCst);
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Denied));
    f.policy.0.store(true, Ordering::SeqCst);
    f.guard
        .set(
            f.grant.capability.task_id(),
            f.grant.capability.attempt_id(),
            AttemptSnapshot::active(Fence::new(2).unwrap()),
        )
        .unwrap();
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Denied));
}
#[test]
fn unknown_outcome_seals_the_attempt_even_with_a_new_call_id() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    f.transport.fail.store(true, Ordering::SeqCst);
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Indeterminate));
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Indeterminate));
    assert_eq!(f.call(ticket.expose(), "b"), Err(RelayError::Indeterminate));
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn spend_reservation_and_freeze_stop_external_calls() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    f.call(ticket.expose(), "a").unwrap();
    f.call(ticket.expose(), "b").unwrap();
    assert_eq!(f.call(ticket.expose(), "c"), Err(RelayError::Budget));
    f.relay.set_frozen(true).unwrap();
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Frozen));
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 2);
}
#[test]
fn publication_failure_never_releases_a_response_or_retries_network() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    f.publisher.0.store(false, Ordering::SeqCst);
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Publication));
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Indeterminate));
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn cancellation_and_expired_lease_close_replay() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    f.call(ticket.expose(), "a").unwrap();
    f.clock.set(500);
    assert_eq!(f.call(ticket.expose(), "a"), Err(RelayError::Denied));
}
#[test]
fn provider_target_model_and_native_agents_are_not_runner_selectable() {
    let f = Fixture::new();
    let ticket = f.relay.issue(f.grant.clone()).unwrap();
    for body in [
        br#"{"model":"other"}"#.as_slice(),
        br#"{"model":"pinned-model","tools":[{"name":"Agent"}]}"#.as_slice(),
        br#"{"model":"pinned-model","tools":[{"type":"web_search"}]}"#.as_slice(),
    ] {
        assert!(
            f.relay
                .relay(
                    ticket.expose(),
                    CallId::parse("a").unwrap(),
                    "/v1/messages",
                    body
                )
                .is_err()
        );
    }
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn opening_gateway_state_always_freezes_admission() {
    let f = Fixture::new();
    let allowed = PermissionSet::new([ToolId::from_static("model.relay")]);
    let reopened = DurableRelay::open(
        &f.directory.path().join("relay.db"),
        f.directory.path().join("gateway-raw"),
        RelayServices {
            guard: f.guard.clone(),
            policy: f.policy.clone(),
            clock: f.clock.clone(),
            audit: Arc::new(MemoryAuditLedger::new()),
            permissions: PermissionLayers::new(allowed.clone(), allowed.clone(), allowed),
            transport: f.transport.clone(),
            publisher: f.publisher.clone(),
        },
    )
    .unwrap();
    assert!(matches!(reopened.issue(f.grant), Err(RelayError::Frozen)));
}
