//! 仅创建（`create-only`）的 `Demo` 环境搭建，外加一次真实的 `HTTP`/`SQLite` -> `Gateway` -> `Stub` -> `Broker` 往返。
//! 不涉及任何供应商、个人登录材料或生产数据源。
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use studiolo::{DemoGateway, DemoGatewayConfig, GatewayStubRunner};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger,
};
use studiolo_control_db::ControlDb;
use studiolo_core::{AttemptId, PermissionSet, TaskId, ToolId, VaultPath};
use studiolo_gateway::{
    AttemptGuard, AttemptGuardError, AttemptSnapshot, Clock, JobCapability, PermissionLayers,
};
use studiolo_runner_adapter::{RunnerAdapter, RunnerObservation};
use studiolo_task_engine::{
    ClaimRequest, PersistentTaskEngine, RetryPolicy, TaskCommand, TaskEngine, TaskSpec,
};
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultReader, VaultWriter};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
struct HostClock;
impl Clock for HostClock {
    fn now_unix_seconds(&self) -> i64 {
        now()
    }
}
struct Guard(Mutex<ControlDb>);
impl AttemptGuard for Guard {
    fn inspect(
        &self,
        task: TaskId,
        attempt: AttemptId,
    ) -> std::result::Result<AttemptSnapshot, AttemptGuardError> {
        let db = self.0.lock().map_err(|_| AttemptGuardError::Unavailable)?;
        let record = db
            .attempt(task, attempt)
            .map_err(|_| AttemptGuardError::Unavailable)?
            .ok_or(AttemptGuardError::UnknownAttempt)?;
        if db
            .is_attempt_active_at(task, attempt, record.fence, now() * 1000)
            .map_err(|_| AttemptGuardError::Unavailable)?
        {
            Ok(AttemptSnapshot::active(record.fence))
        } else {
            Ok(AttemptSnapshot::cancelled(record.fence))
        }
    }
}
#[derive(Clone)]
struct Ledger(Arc<SqliteAuditLedger>);
impl AuditLedger for Ledger {
    fn append(&self, e: AuditEvent) -> std::result::Result<AuditRecord, AuditError> {
        self.0.append(e)
    }
    fn query(&self, f: &AuditFilter) -> std::result::Result<Vec<AuditRecord>, AuditError> {
        self.0.query(f)
    }
}
struct DemoWeb {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl DemoWeb {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let mut header = Vec::new();
                        let mut byte = [0];
                        while header.len() < 8192 && !header.ends_with(b"\r\n\r\n") {
                            match stream.read(&mut byte) {
                                Ok(1) => header.push(byte[0]),
                                _ => break,
                            }
                        }
                        let valid = header.starts_with(b"GET /demo.json HTTP/1.1\r\n")
                            && header.ends_with(b"\r\n\r\n");
                        let (status, body) = if valid {
                            (
                                "200 OK",
                                r#"{"mode":"demo","headline":"Local demonstration source","value":7}"#,
                            )
                        } else {
                            ("404 Not Found", "{}")
                        };
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for DemoWeb {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn tools() -> PermissionSet {
    PermissionSet::new([
        ToolId::from_static("web.fetch"),
        ToolId::from_static("db.query"),
    ])
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.len() > 2 || (args.len() == 2 && args[1] != "--keep-web") {
        return Err("usage: demo NEW_ABSOLUTE_DIRECTORY [--keep-web]".into());
    }
    let destination = PathBuf::from(&args[0]);
    if !destination.is_absolute() {
        return Err("Demo directory must be absolute and new".into());
    }
    let parent = destination
        .parent()
        .ok_or("Demo parent missing")?
        .canonicalize()?;
    let root = parent.join(
        destination
            .file_name()
            .ok_or("Demo directory name missing")?,
    );
    // 绝不覆盖或接管已有的用户目录；出错时保留已完成的部分搭建。
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    for name in ["broker", "control", "audit", "sources"] {
        fs::DirBuilder::new().mode(0o700).create(root.join(name))?;
    }
    let database = root.join("sources/demo.sqlite3");
    let db = rusqlite::Connection::open(&database)?;
    db.execute_batch("CREATE TABLE demo_items(id INTEGER PRIMARY KEY,label TEXT NOT NULL,value INTEGER NOT NULL); INSERT INTO demo_items VALUES(1,'Demo metric',7);")?;
    drop(db);
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600))?;
    let mut broker = FsVaultBroker::open(&root.join("vault"), &root.join("broker"))?;
    let input = VaultPath::parse("项目/Demo/00-总览/INDEX.md")?;
    broker.mutate(Mutation::create(
        input.clone(),
        b"# Demo\n\nCurrent sources: DEMO only. No model keys or production sources.\n".to_vec(),
        "demo-setup",
    ))?;
    let uuid = broker.vault_uuid();
    let control = root.join("control/control.sqlite3");
    let audit = Arc::new(SqliteAuditLedger::open(&root.join("audit"))?);
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&control, uuid)?,
        broker,
        Ledger(audit.clone()),
    );
    let task = engine.create_task(
        TaskSpec {
            role: "demo".into(),
            adapter: "stub".into(),
            input_paths: vec![input],
            allow_web: vec!["demo-web".into()],
            allow_db: vec!["demo-db".into()],
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff_seconds: 0,
                retry_on: vec![],
            },
            timeout: 300,
            project: "Demo".into(),
            parent: None,
            title: Some("Demo source round trip".into()),
            instruction: Some(
                "Read the two demonstration sources and retain reviewable candidates.".into(),
            ),
        },
        now() * 1000,
    )?;
    engine.command(task, TaskCommand::Run, now() * 1000)?;
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "demo-stub".into(),
            now_ms: now() * 1000,
            lease_until_ms: (now() + 120) * 1000,
        })?
        .ok_or("Demo task did not claim")?;
    let web = DemoWeb::start()?;
    let gateway = Arc::new(DemoGateway::open(
        DemoGatewayConfig {
            mode: "demo".into(),
            endpoint: web.address,
            database,
        },
        Arc::new(Guard(Mutex::new(ControlDb::open(&control, uuid)?))),
        audit.clone(),
        Arc::new(HostClock),
        PermissionLayers::new(tools(), tools(), tools()),
    )?);
    let spec = &claimed.run_spec;
    let runner = GatewayStubRunner::new(
        gateway,
        JobCapability::new(task, spec.attempt_id, spec.fence, tools(), now() + 120),
    );
    let started = runner.start(spec)?;
    engine.attach_runner(
        task,
        spec.attempt_id,
        spec.fence,
        started.runner_ref,
        now() * 1000,
    )?;
    let RunnerObservation::Completed(report) = runner.observe(&started.handle)? else {
        return Err("Demo did not return candidates".into());
    };
    let result = engine.report("demo-source-roundtrip", report, now() * 1000)?;
    engine.project_pending()?;
    println!("当前为演示源：本机 HTTP 与只读 SQLite。候选已封存，未代替人工验收。");
    println!(
        "{}",
        serde_json::json!({"mode":"demo","root":root,"task_id":task,"status":engine.task(task)?.ok_or("task missing")?.status,"manifest":result.receipt.manifest_path,"audit_events":audit.query(&AuditFilter::default())?.len(),"demo_web":web.address.to_string(),"web_running_until_input":args.len()==2,"viewer_argv":["studiolo-local-server","--vault",root.join("vault"),"--assets","web/dist","--label","当前为演示源","--port","4317"]})
    );
    println!("人侧只读示例参数见 viewer_argv；其中路径是独立参数，不能直接拼接成 shell 命令。");
    if args.len() == 2 {
        println!("Demo Web 已启动；按 Enter 停止。数据与候选将保留。");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
    }
    Ok(())
}
