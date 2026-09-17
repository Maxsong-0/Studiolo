//! 临时双库和公开组合 Router；重开时不保留旧数据库句柄。

use super::*;
use crate::test_support::{claims, signed, verifier};
use axum::{Router, body::Body, http::Request, response::Response};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};
use studiolo_auth::{AccessIdentity, OsSessionRandom, SessionManager, Totp};
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

pub struct Fixture {
    _root: TempDir,
    pub root: PathBuf,
    pub auth: PathBuf,
    pub audit: PathBuf,
    pub clock: Arc<Clock>,
    pub app: Router,
}

impl Fixture {
    pub async fn new(enrolled: bool) -> Self {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let auth = root.join("auth");
        let audit = root.join("audit");
        for path in [&root, &auth, &audit, &root.join("vault"), &root.join("web")] {
            if !path.exists() {
                fs::create_dir(path).unwrap();
            }
            mode(path, 0o700);
        }
        fs::create_dir(root.join("web/assets")).unwrap();
        fs::write(root.join("vault/a.md"), "# 示例\nreading\n").unwrap();
        fs::write(
            root.join("web/index.html"),
            "<!doctype html><title>虚构夹具</title>",
        )
        .unwrap();
        fs::write(root.join("web/assets/app.js"), "const fixture = true;").unwrap();
        fs::write(root.join("web/assets/app.css"), "body { color: black; }").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let clock = Arc::new(Clock(AtomicI64::new(now)));
        let ledger = Arc::new(SqliteAuditLedger::open(&audit).unwrap());
        let sessions = Arc::new(
            SessionManager::migrate_v3(
                &auth,
                config(),
                ledger,
                clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        if enrolled {
            let (_, enrollment) = Totp::attach(sessions.clone(), seed()).unwrap();
            enrollment.enroll(code(now)).unwrap();
        }
        let app = build(&root, sessions).await;
        Self {
            _root: temporary,
            root,
            auth,
            audit,
            clock,
            app,
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    pub fn advance(&self, milliseconds: i64) {
        self.clock.0.fetch_add(milliseconds, Ordering::SeqCst);
    }

    pub fn manager(&self) -> Arc<SessionManager> {
        let ledger = Arc::new(SqliteAuditLedger::open_existing(&self.audit).unwrap());
        Arc::new(
            SessionManager::open_existing(
                &self.auth,
                config(),
                ledger,
                self.clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        )
    }

    pub async fn reopen(&mut self) {
        // 释放 HTTP 持有的全部连接，再从同一临时双库重新装配。
        self.app = Router::new();
        self.app = build(&self.root, self.manager()).await;
    }

    pub async fn identity(&self) -> AccessIdentity {
        verifier().await.verify(&signed(&claims())).await.unwrap()
    }

    pub fn request(
        &self,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:4319")
            .header("cf-access-jwt-assertion", signed(&claims()))
            .header("x-studiolo-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty");
        if method == "POST" {
            builder = builder
                .header("origin", ORIGIN)
                .header("content-type", "application/json");
        }
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    pub async fn send(&self, request: Request<Body>, status: u16) -> Response {
        let response = self.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()["vary"], "Origin, Sec-Fetch-Site");
        secure(&response);
        response
    }

    pub async fn issue(&self) -> Issued {
        let response = self
            .send(self.request("POST", SESSIONS, None, "{}"), 201)
            .await;
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        cookie_attributes(&cookie);
        let cookie = cookie.split(';').next().unwrap().to_owned();
        let pair = cookie.strip_prefix(&format!("{COOKIE}=")).unwrap();
        let (token, csrf) = pair.split_once('~').unwrap();
        let token = token.to_owned();
        let csrf = csrf.to_owned();
        let body = json(response).await;
        assert_eq!(body["csrf_token"], csrf);
        assert_eq!(body["session"]["state"], "active");
        assert!(!body.to_string().contains(&token));
        assert_eq!(body.as_object().unwrap().len(), 3);
        let owner_scope = body["owner_scope"].as_str().unwrap();
        assert_eq!(owner_scope.len(), 64);
        assert!(
            owner_scope
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(body["session"].as_object().unwrap().len(), 5);
        Issued {
            cookie,
            csrf,
            token,
            id: body["session"]["id"].as_str().unwrap().into(),
        }
    }

    pub fn events(&self, action: &str) -> Vec<AuditRecord> {
        SqliteAuditLedger::open_existing(&self.audit)
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .into_iter()
            .filter(|record| record.event().action() == action)
            .collect()
    }

    pub fn pending_enrollment(&self) {
        let (_, enrollment) = Totp::attach(self.manager(), seed()).unwrap();
        let path = self.audit.join("audit.sqlite3");
        mode(&path, 0o400);
        let result = enrollment.enroll(code(self.now()));
        mode(&path, 0o600);
        assert_eq!(result.unwrap_err(), SessionError::AuditUnavailable);
    }
}

async fn build(root: &Path, sessions: Arc<SessionManager>) -> Router {
    studiolo_local_server::router_with_sessions(
        ReadOnlyVault::open(&root.join("vault")).unwrap(),
        ReadOnlyVault::open(&root.join("web")).unwrap(),
        "虚构示例".into(),
        4319,
        verifier().await,
        sessions,
    )
    .unwrap()
}
