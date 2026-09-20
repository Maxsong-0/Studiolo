//! Memory seam and real isolated Demo HTTP/SQLite evidence for the M6 synthetic runner.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use studiolo::{DemoGateway, DemoGatewayConfig, GatewayStubRunner};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_core::{AttemptId, Fence, PermissionSet, TaskId, ToolId};
use studiolo_gateway::{
    AttemptSnapshot, JobCapability, ManualClock, MemoryAttemptGuard, MemoryGateway,
    PermissionLayers,
};
use studiolo_runner_adapter::{
    RunSpec, RunnerAdapter, RunnerError, RunnerObservation, RunnerReport,
};

fn tools() -> PermissionSet {
    PermissionSet::new([
        ToolId::from_static("web.fetch"),
        ToolId::from_static("db.query"),
    ])
}
fn spec() -> RunSpec {
    RunSpec {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        instruction: "Collect the two Demo source responses for review".into(),
        inputs: vec![],
        budget: Default::default(),
        execution: None,
    }
}
fn capability(spec: &RunSpec) -> JobCapability {
    JobCapability::new(spec.task_id, spec.attempt_id, spec.fence, tools(), 2000)
}
fn activate(guard: &MemoryAttemptGuard, spec: &RunSpec) {
    guard
        .set(
            spec.task_id,
            spec.attempt_id,
            AttemptSnapshot::active(spec.fence),
        )
        .unwrap();
}
fn report(runner: &GatewayStubRunner, spec: &RunSpec) -> RunnerReport {
    let started = runner.start(spec).unwrap();
    match runner.observe(&started.handle).unwrap() {
        RunnerObservation::Completed(report) => report,
        other => panic!("unexpected {other:?}"),
    }
}
fn body(report: &RunnerReport) -> Value {
    serde_json::from_slice(
        &report
            .artifacts
            .iter()
            .find(|a| a.name.as_str() == "gateway-demo.json")
            .unwrap()
            .content,
    )
    .unwrap()
}
fn memory(
    allow: bool,
) -> (
    Arc<MemoryGateway>,
    Arc<MemoryAttemptGuard>,
    Arc<MemoryAuditLedger>,
    Arc<ManualClock>,
) {
    let guard = Arc::new(MemoryAttemptGuard::new());
    let audit = Arc::new(MemoryAuditLedger::new());
    let clock = Arc::new(ManualClock::new(1000));
    let gateway = Arc::new(MemoryGateway::new(
        guard.clone(),
        audit.clone(),
        clock.clone(),
        PermissionLayers::new(
            tools(),
            if allow {
                tools()
            } else {
                PermissionSet::default()
            },
            tools(),
        ),
    ));
    gateway
        .set_tool_response(
            ToolId::from_static("web.fetch"),
            json!({"weather":"sun","n":1}),
        )
        .unwrap();
    gateway
        .set_tool_response(
            ToolId::from_static("db.query"),
            json!({"rows":[{"count":3}]}),
        )
        .unwrap();
    (gateway, guard, audit, clock)
}
#[test]
fn memory_gateway_values_determine_candidates_and_stable_calls_replay_original_results() {
    let (gateway, guard, audit, _) = memory(true);
    let mut input = spec();
    activate(&guard, &input);
    let runner = GatewayStubRunner::new(gateway.clone(), capability(&input));
    let first = report(&runner, &input);
    let first_body = body(&first);
    assert_eq!(
        first_body["results"][0]["output"],
        json!({"weather":"sun","n":1})
    );
    assert_eq!(
        first_body["results"][1]["output"],
        json!({"rows":[{"count":3}]})
    );
    gateway
        .set_tool_response(
            ToolId::from_static("web.fetch"),
            json!({"weather":"rain","n":2}),
        )
        .unwrap();
    gateway
        .set_tool_response(
            ToolId::from_static("db.query"),
            json!({"rows":[{"count":9}]}),
        )
        .unwrap();
    assert_eq!(report(&runner, &input), first);
    assert_eq!(
        audit
            .query(&AuditFilter::default().with_attempt(input.attempt_id))
            .unwrap()
            .len(),
        4
    );
    input.attempt_id = AttemptId::new();
    activate(&guard, &input);
    let next = GatewayStubRunner::new(gateway, capability(&input));
    let second = body(&report(&next, &input));
    assert_eq!(
        second["results"][0]["output"],
        json!({"weather":"rain","n":2})
    );
    assert_eq!(
        second["results"][1]["output"],
        json!({"rows":[{"count":9}]})
    );
    assert_ne!(first_body["results"], second["results"]);
}
#[test]
fn denied_revoked_expired_and_wrong_identity_attempts_cannot_release_candidates() {
    let partial_guard = Arc::new(MemoryAttemptGuard::new());
    let partial_spec = spec();
    activate(&partial_guard, &partial_spec);
    let partial = Arc::new(MemoryGateway::new(
        partial_guard,
        Arc::new(MemoryAuditLedger::new()),
        Arc::new(ManualClock::new(1000)),
        PermissionLayers::new(tools(), tools(), tools()),
    ));
    partial
        .set_tool_response(
            ToolId::from_static("web.fetch"),
            json!({"partial":"web only"}),
        )
        .unwrap();
    assert!(
        GatewayStubRunner::new(partial, capability(&partial_spec))
            .start(&partial_spec)
            .is_err()
    );
    let input = spec();
    let (gateway, guard, _, _) = memory(false);
    activate(&guard, &input);
    let denied = GatewayStubRunner::new(gateway, capability(&input));
    assert!(denied.start(&input).is_err());
    let (gateway, guard, _, clock) = memory(true);
    activate(&guard, &input);
    let runner = GatewayStubRunner::new(gateway, capability(&input));
    let started = runner.start(&input).unwrap();
    let mut changed = input.clone();
    changed.instruction.push_str(" changed");
    assert!(matches!(
        runner.start(&changed),
        Err(RunnerError::IdempotencyConflict)
    ));
    changed = input.clone();
    changed.fence = Fence::new(2).unwrap();
    assert!(matches!(
        runner.start(&changed),
        Err(RunnerError::InvalidExecution)
    ));
    guard
        .set(
            input.task_id,
            input.attempt_id,
            AttemptSnapshot::cancelled(input.fence),
        )
        .unwrap();
    assert!(runner.observe(&started.handle).is_err());
    assert!(runner.start(&input).is_err());
    activate(&guard, &input);
    clock.set(2000);
    assert!(runner.observe(&started.handle).is_err());
    clock.set(1000);
    runner.cancel(&started.handle).unwrap();
    assert_eq!(
        runner.observe(&started.handle).unwrap(),
        RunnerObservation::Cancelled
    );
}

struct Server {
    address: SocketAddr,
    value: Arc<Mutex<Value>>,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let value = Arc::new(Mutex::new(json!({"headline":"first real Demo page"})));
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (v, h, s) = (value.clone(), hits.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            request.push(byte[0]);
                        }
                        assert!(request.starts_with(b"GET /demo.json HTTP/1.1\r\n"));
                        h.fetch_add(1, Ordering::SeqCst);
                        let body = serde_json::to_vec(&*v.lock().unwrap()).unwrap();
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        stream.write_all(headers.as_bytes()).unwrap();
                        stream.write_all(&body).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        Self {
            address,
            value,
            hits,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
fn database() -> (tempfile::TempDir, PathBuf, rusqlite::Connection) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("demo.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE demo_items(id INTEGER PRIMARY KEY,label TEXT NOT NULL,value INTEGER NOT NULL);INSERT INTO demo_items VALUES(1,'demo count',3);").unwrap();
    (temp, path, db)
}
#[test]
fn real_demo_http_and_readonly_sqlite_changes_reach_new_attempt_candidates() {
    let server = Server::new();
    let (_temp, path, db) = database();
    let guard = Arc::new(MemoryAttemptGuard::new());
    let audit = Arc::new(MemoryAuditLedger::new());
    let clock = Arc::new(ManualClock::new(1000));
    let gateway = Arc::new(
        DemoGateway::open(
            DemoGatewayConfig {
                mode: "demo".into(),
                endpoint: server.address,
                database: path,
            },
            guard.clone(),
            audit.clone(),
            clock,
            PermissionLayers::new(tools(), tools(), tools()),
        )
        .unwrap(),
    );
    let mut input = spec();
    activate(&guard, &input);
    let runner = GatewayStubRunner::new(gateway.clone(), capability(&input));
    let first = body(&report(&runner, &input));
    assert_eq!(
        first["results"][0]["output"]["headline"],
        "first real Demo page"
    );
    assert_eq!(first["results"][1]["output"]["rows"][0]["value"], 3);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    *server.value.lock().unwrap() = json!({"headline":"changed real Demo page"});
    db.execute("UPDATE demo_items SET value=9", []).unwrap();
    assert_eq!(body(&report(&runner, &input)), first);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    input.attempt_id = AttemptId::new();
    activate(&guard, &input);
    let next = GatewayStubRunner::new(gateway, capability(&input));
    let second = body(&report(&next, &input));
    assert_eq!(
        second["results"][0]["output"]["headline"],
        "changed real Demo page"
    );
    assert_eq!(second["results"][1]["output"]["rows"][0]["value"], 9);
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);
    let effect = audit
        .query(&AuditFilter::default().with_action("gateway.demo.effect"))
        .unwrap();
    assert_eq!(effect.len(), 8);
    assert!(effect.iter().all(|r| {
        !r.event()
            .payload()
            .as_value()
            .to_string()
            .contains("headline")
    }));
}
#[test]
fn real_demo_denial_happens_before_http_and_unsafe_targets_or_arbitrary_queries_are_rejected() {
    use studiolo_gateway::{CallId, Gateway, ToolCall};
    let server = Server::new();
    let (_temp, path, _db) = database();
    let guard = Arc::new(MemoryAttemptGuard::new());
    let audit = Arc::new(MemoryAuditLedger::new());
    let clock = Arc::new(ManualClock::new(1000));
    for (mode, address) in [
        ("production", server.address),
        ("demo", "192.0.2.1:80".parse().unwrap()),
    ] {
        assert!(
            DemoGateway::open(
                DemoGatewayConfig {
                    mode: mode.into(),
                    endpoint: address,
                    database: path.clone()
                },
                guard.clone(),
                audit.clone(),
                clock.clone(),
                PermissionLayers::new(tools(), tools(), tools())
            )
            .is_err()
        );
    }
    let gateway = Arc::new(
        DemoGateway::open(
            DemoGatewayConfig {
                mode: "demo".into(),
                endpoint: server.address,
                database: path.clone(),
            },
            guard.clone(),
            audit.clone(),
            clock.clone(),
            PermissionLayers::new(tools(), PermissionSet::default(), tools()),
        )
        .unwrap(),
    );
    let input = spec();
    activate(&guard, &input);
    let runner = GatewayStubRunner::new(gateway, capability(&input));
    assert!(runner.start(&input).is_err());
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    let gateway = DemoGateway::open(
        DemoGatewayConfig {
            mode: "demo".into(),
            endpoint: server.address,
            database: path,
        },
        guard,
        audit,
        clock,
        PermissionLayers::new(tools(), tools(), tools()),
    )
    .unwrap();
    let call = ToolCall::new(
        CallId::parse("unsafe-url").unwrap(),
        ToolId::from_static("web.fetch"),
        json!({"url":"http://example.com"}),
    );
    assert!(gateway.invoke(&capability(&input), call).is_err());
    let call = ToolCall::new(
        CallId::parse("unsafe-sql").unwrap(),
        ToolId::from_static("db.query"),
        json!({"sql":"DROP TABLE demo_items"}),
    );
    assert!(gateway.invoke(&capability(&input), call).is_err());
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
}
