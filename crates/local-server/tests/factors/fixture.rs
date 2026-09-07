//! 经公开组合和真实Unix服务创建夹具；不伪造连接或网络来源证明。

use super::{support::*, test_support::verifier};
use axum::Router;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};
use studiolo_auth::{FactorRuntime, OsSessionRandom, SessionManager, Totp, TotpCode};
use studiolo_local_server::PreparedUnixIngress;
use studiolo_vault_broker::ReadOnlyVault;

pub struct Fixture {
    _temporary: tempfile::TempDir,
    pub root: PathBuf,
    pub socket: PathBuf,
    pub clock: Arc<Clock>,
    pub audit_override: Option<Arc<dyn AuditLedger>>,
    replacement: bool,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl Fixture {
    pub async fn new(replacement: bool) -> Self {
        // macOS默认临时根过长；先解析系统短路径，再创建可独立清理的私有目录。
        let short_root = std::path::Path::new("/tmp").canonicalize().unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("sf-")
            .tempdir_in(short_root)
            .unwrap();
        let root = temporary.path().canonicalize().unwrap();
        mode(&root, 0o700);
        for directory in ["auth", "audit", "vault", "web", "run"] {
            let path = root.join(directory);
            fs::create_dir(&path).unwrap();
            mode(&path, 0o700);
        }
        fs::write(root.join("vault/note.md"), "# 虚构笔记\n").unwrap();
        fs::write(
            root.join("web/index.html"),
            "<!doctype html><title>测试</title>",
        )
        .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let clock = Arc::new(Clock(AtomicI64::new(now)));
        let audit = Arc::new(SqliteAuditLedger::open(&root.join("audit")).unwrap());
        let manager = Arc::new(
            SessionManager::migrate_v3(
                &root.join("auth"),
                config(),
                audit,
                clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        let (_, enrollment) = Totp::attach(manager.clone(), seed(false)).unwrap();
        enrollment
            .enroll(TotpCode::parse(otp(&OLD, now)).unwrap())
            .unwrap();
        drop(enrollment);
        drop(manager);
        clock.0.fetch_add(60_000, Ordering::SeqCst);
        let socket = root.join("run/s");
        let mut fixture = Self {
            _temporary: temporary,
            root,
            socket,
            clock,
            audit_override: None,
            replacement,
            shutdown: None,
            server: None,
        };
        fixture.start().await;
        fixture
    }

    pub fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    pub fn advance(&self, milliseconds: i64) {
        self.clock.0.fetch_add(milliseconds, Ordering::SeqCst);
    }

    pub fn manager(&self) -> Arc<SessionManager> {
        let audit = self.audit_override.clone().unwrap_or_else(|| {
            Arc::new(SqliteAuditLedger::open_existing(&self.root.join("audit")).unwrap())
        });
        Arc::new(
            SessionManager::open_existing(
                &self.root.join("auth"),
                config(),
                audit,
                self.clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        )
    }

    pub async fn router(&self) -> Router {
        let sessions = self.manager();
        let factors = Arc::new(
            FactorRuntime::attach(
                sessions.clone(),
                seed(false),
                self.replacement.then(|| seed(true)),
            )
            .unwrap(),
        );
        studiolo_local_server::router_with_factors(
            (ReadOnlyVault::open(&self.root.join("vault")).unwrap(), None),
            ReadOnlyVault::open(&self.root.join("web")).unwrap(),
            "虚构示例".into(),
            verifier().await,
            sessions,
            factors,
        )
        .unwrap()
    }

    async fn start(&mut self) {
        let isolated: Vec<_> = ["auth", "audit", "vault", "web"]
            .map(|name| self.root.join(name))
            .into();
        let roots: Vec<_> = isolated.iter().map(PathBuf::as_path).collect();
        let bound = PreparedUnixIngress::prepare(&self.socket, &roots)
            .unwrap()
            .bind()
            .unwrap();
        let app = self.router().await;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.shutdown = Some(sender);
        self.server = Some(tokio::spawn(async move {
            bound
                .serve_shutdown(app, async move {
                    let _ = receiver.await;
                })
                .await
                .unwrap();
        }));
    }

    pub async fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).unwrap();
        }
        if let Some(server) = self.server.take() {
            tokio::time::timeout(Duration::from_secs(6), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    pub async fn restart(&mut self) {
        self.stop().await;
        assert!(!self.socket.exists());
        self.start().await;
    }

    pub fn events(&self, action: &str) -> Vec<AuditRecord> {
        SqliteAuditLedger::open_existing(&self.root.join("audit"))
            .unwrap()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
    }
}
