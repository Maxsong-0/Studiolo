//! M4 人侧策略端点契约（真实 Unix 入口、私有双 SQLite、虚构签名身份、真实 TOTP）：
//! desired → 校验／diff → 发布挡在 L3 后 → effective；回滚；三级吊销与解除；方法与查询封闭；
//! 同一对象只能发布最新的未发布草案（T-181）；未装配策略平面时 404。用例 36 的人侧一半。

#[allow(dead_code)]
#[path = "factors/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "factors/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{Router, http::Request};
use client::{Client, Reply};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};
use studiolo_auth::{FactorRuntime, OsSessionRandom, SessionManager, Totp, TotpCode};
use studiolo_local_server::{PolicyPlane, PreparedUnixIngress, with_policy_plane};
use studiolo_policy::{DesiredStatus, PolicyStore};
use studiolo_vault_broker::ReadOnlyVault;
use support::*;
use test_support::{claims, signed, verifier};

const STATUS: &str = "/api/v1/policy";
const DESIRED: &str = "/api/v1/policy/desired";
const VALIDATE: &str = "/api/v1/policy/validate";
const ACTIVATE: &str = "/api/v1/policy/activate";
const VERSIONS: &str = "/api/v1/policy/versions";
const REVOKE: &str = "/api/v1/agents/revoke";

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    socket: PathBuf,
    clock: Arc<Clock>,
    plane: bool,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

struct Login {
    cookie: String,
    csrf: String,
}

impl Fixture {
    async fn new(plane: bool) -> Self {
        let short_root = std::path::Path::new("/tmp").canonicalize().unwrap();
        let temporary = tempfile::Builder::new()
            .prefix("sp-")
            .tempdir_in(short_root)
            .unwrap();
        let root = temporary.path().canonicalize().unwrap();
        mode(&root, 0o700);
        for directory in ["auth", "audit", "vault", "web", "run", "policy"] {
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
            plane,
            shutdown: None,
            server: None,
        };
        fixture.start().await;
        fixture
    }

    fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    fn manager(&self) -> Arc<SessionManager> {
        let audit = Arc::new(SqliteAuditLedger::open_existing(&self.root.join("audit")).unwrap());
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

    fn policy(&self) -> PolicyStore {
        PolicyStore::open_existing(&self.root.join("policy")).unwrap()
    }

    async fn router(&self) -> Router {
        let sessions = self.manager();
        let factors = Arc::new(FactorRuntime::attach(sessions.clone(), seed(false), None).unwrap());
        let app = studiolo_local_server::router_with_factors(
            (ReadOnlyVault::open(&self.root.join("vault")).unwrap(), None),
            ReadOnlyVault::open(&self.root.join("web")).unwrap(),
            "虚构示例".into(),
            verifier().await,
            sessions,
            factors,
        )
        .unwrap();
        if self.plane {
            let ledger: Arc<dyn AuditLedger> =
                Arc::new(SqliteAuditLedger::open_existing(&self.root.join("audit")).unwrap());
            with_policy_plane(
                app,
                Arc::new(PolicyPlane::new(
                    PolicyStore::open(&self.root.join("policy")).unwrap(),
                    ledger,
                )),
            )
        } else {
            app
        }
    }

    async fn start(&mut self) {
        let isolated: Vec<_> = ["auth", "audit", "vault", "web", "policy"]
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

    async fn stop(&mut self) {
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

    fn events(&self, action: &str) -> Vec<AuditRecord> {
        SqliteAuditLedger::open_existing(&self.root.join("audit"))
            .unwrap()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        login: Option<&Login>,
        body: &str,
    ) -> Request<Vec<u8>> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", HOST)
            .header("origin", ORIGIN)
            .header("cf-access-jwt-assertion", signed(&claims()))
            .header("cf-connecting-ip", "192.0.2.1")
            .header("x-studiolo-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty")
            .header("content-type", "application/json");
        if let Some(login) = login {
            request = request
                .header("cookie", &login.cookie)
                .header("x-csrf-token", &login.csrf);
        }
        request.body(body.as_bytes().to_vec()).unwrap()
    }

    async fn send(&self, request: Request<Vec<u8>>, status: u16) -> Reply {
        let reply = Client::connect(&self.socket).await.send(request).await;
        assert_eq!(
            reply.status,
            status,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        reply.secure();
        reply
    }

    async fn issue(&self) -> Login {
        let response = self
            .send(self.request("POST", SESSIONS, None, "{}"), 201)
            .await;
        let raw = response.headers["set-cookie"].to_str().unwrap();
        let cookie = raw.split(';').next().unwrap().to_owned();
        let csrf = response.json()["csrf_token"].as_str().unwrap().to_owned();
        Login { cookie, csrf }
    }

    async fn elevate(&self, login: &Login) {
        let code = otp(&OLD, self.now());
        self.send(
            self.request(
                "POST",
                STEP_UP,
                Some(login),
                &format!("{{\"code\":\"{code}\"}}"),
            ),
            200,
        )
        .await;
    }

    async fn status(&self, login: &Login) -> Value {
        self.send(self.request("GET", STATUS, Some(login), ""), 200)
            .await
            .json()
    }
}

fn capability(whitelist: &[&str], propose: bool) -> String {
    json!({
        "ai_type": "claude",
        "read": {"project_whitelist": whitelist, "deny_by_default": true},
        "write": {"board": false, "content_patch_only": true, "content_direct": false},
        "policy": {"propose_desired": propose, "activate": false},
        "hard_bound_skills": [],
    })
    .to_string()
}

#[tokio::test]
async fn desired_is_validated_diffed_and_activation_is_gated_behind_l3() {
    let mut f = Fixture::new(true).await;
    // 未登录：401；登录后 effective 为空。
    f.send(f.request("GET", STATUS, None, ""), 401).await;
    let login = f.issue().await;
    let status = f.status(&login).await;
    let initial = status["effective_hash"].as_str().unwrap().to_owned();
    assert_eq!(initial.len(), 64);
    assert_eq!(status["entries"].as_array().unwrap().len(), 0);
    assert_eq!(status["l3_available"], true);
    assert_eq!(status["global_revoked"], false);

    // 提交 desired：校验通过、扩大项标出、effective 不变。
    let submitted = f
        .send(
            f.request(
                "POST",
                DESIRED,
                Some(&login),
                &json!({"kind": "capability_set", "subject": "claude", "body": capability(&["项目/Studiolo"], true)}).to_string(),
            ),
            200,
        )
        .await
        .json();
    let desired = &submitted["desired"];
    assert_eq!(desired["status"], "validated");
    assert_eq!(desired["report"]["ok"], true);
    assert_eq!(desired["report"]["base_effective_hash"], initial);
    let expansions = desired["report"]["expansions"].as_array().unwrap();
    assert_eq!(expansions.len(), 3, "{expansions:?}");
    assert_eq!(desired["report"]["diff"]["format"], "json");
    let desired_id = desired["id"].as_str().unwrap().to_owned();
    assert_eq!(f.status(&login).await["effective_hash"], initial);
    assert_eq!(f.events("policy.desired_submitted").len(), 1);

    // 用例 36：没有近期认证 → 403，版本不变，拒绝进审计（Intent 已先落账）。
    let activate =
        json!({"desired_id": desired_id, "expected_effective_hash": initial}).to_string();
    let denied = f
        .send(f.request("POST", ACTIVATE, Some(&login), &activate), 403)
        .await
        .json();
    assert_eq!(denied["code"], "reauthentication_required");
    assert_eq!(f.status(&login).await["effective_hash"], initial);
    let refusals = f.events("policy.activate_denied");
    assert_eq!(refusals.len(), 1);
    assert_eq!(
        refusals[0].event().payload().as_value()["reason"],
        "reauthentication_required"
    );
    assert_eq!(f.events("policy.activated").len(), 1, "只有 Intent");

    // 二次认证后发布：Intent + Outcome 同 id；effective 变化；desired 转 activated。
    f.elevate(&login).await;
    let published = f
        .send(f.request("POST", ACTIVATE, Some(&login), &activate), 200)
        .await
        .json();
    let activation = &published["activation"];
    assert_eq!(activation["version"], 1);
    assert_eq!(activation["previous_effective_hash"], initial);
    assert_eq!(published["audit_confirmed"], true);
    let effective = activation["effective_hash"].as_str().unwrap().to_owned();
    assert_ne!(effective, initial);
    let activated = f.events("policy.activated");
    assert_eq!(activated.len(), 3);
    assert_eq!(
        activated[1].event().event_id(),
        activated[2].event().event_id()
    );
    assert_eq!(
        activated[2].event().payload().as_value()["effective_hash"],
        effective
    );
    let status = f.status(&login).await;
    assert_eq!(status["effective_hash"], effective);
    assert_eq!(status["entries"][0]["subject"], "claude");
    // 同一 desired 不能二次发布；过期哈希 409 并回传当前哈希。
    f.send(f.request("POST", ACTIVATE, Some(&login), &activate), 409)
        .await;
    let stale = f
        .send(
            f.request(
                "POST",
                ACTIVATE,
                Some(&login),
                &json!({"rollback_to": 1, "expected_effective_hash": initial}).to_string(),
            ),
            409,
        )
        .await
        .json();
    assert_eq!(stale["code"], "hash_mismatch");
    assert_eq!(stale["current_effective_hash"], effective);

    // 校验被拒的 desired：422，不产生版本。
    let bad = f
        .send(
            f.request(
                "POST",
                DESIRED,
                Some(&login),
                &json!({"kind": "capability_set", "subject": "claude", "body": "{}"}).to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(bad["desired"]["status"], "rejected");
    let bad_id = bad["desired"]["id"].as_str().unwrap().to_owned();
    f.send(
        f.request(
            "POST",
            ACTIVATE,
            Some(&login),
            &json!({"desired_id": bad_id, "expected_effective_hash": effective}).to_string(),
        ),
        422,
    )
    .await;
    // 收窄的新版本 + 回滚到 v1 = v3。
    let narrowed = f
        .send(
            f.request(
                "POST",
                DESIRED,
                Some(&login),
                &json!({"kind": "capability_set", "subject": "claude", "body": capability(&["项目/Studiolo"], false)}).to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(
        narrowed["desired"]["report"]["narrowings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let narrowed_id = narrowed["desired"]["id"].as_str().unwrap().to_owned();
    let revalidated = f
        .send(
            f.request(
                "POST",
                VALIDATE,
                Some(&login),
                &json!({"desired_id": narrowed_id}).to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(revalidated["desired"]["status"], "validated");
    assert_eq!(f.events("policy.validated").len(), 1);
    let v2 = f
        .send(
            f.request(
                "POST",
                ACTIVATE,
                Some(&login),
                &json!({"desired_id": narrowed_id, "expected_effective_hash": effective})
                    .to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(v2["activation"]["version"], 2);
    let hash2 = v2["activation"]["effective_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let v3 = f
        .send(
            f.request(
                "POST",
                ACTIVATE,
                Some(&login),
                &json!({"rollback_to": 1, "expected_effective_hash": hash2}).to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(v3["activation"]["version"], 3);
    assert_eq!(v3["activation"]["rollback_of"], 1);
    let versions = f
        .send(f.request("GET", VERSIONS, Some(&login), ""), 200)
        .await
        .json();
    assert_eq!(
        versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|meta| meta["version"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );
    let first = f
        .send(
            f.request("GET", &format!("{VERSIONS}/1"), Some(&login), ""),
            200,
        )
        .await
        .json();
    assert!(
        first["version"]["body"]
            .as_str()
            .unwrap()
            .contains("项目/Studiolo")
    );
    f.send(
        f.request("GET", &format!("{VERSIONS}/99"), Some(&login), ""),
        404,
    )
    .await;
    // 策略库直读核对：人侧发布的就是 effective。
    assert_eq!(f.policy().effective().unwrap().entries.len(), 1);
    f.stop().await;
}

#[tokio::test]
async fn an_older_desired_cannot_be_published_while_a_newer_one_is_open() {
    // T-181／BUG-031：同一对象有两份未发布草案时，发布较早那份被拒、什么都不写；
    // 列表为每份未发布草案列出比它更新的那几份，与发布时的拦截同一口径。
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let initial = f.status(&login).await["effective_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut ids = Vec::new();
    for propose in [true, false] {
        let submitted = f
            .send(
                f.request(
                    "POST",
                    DESIRED,
                    Some(&login),
                    &json!({"kind": "capability_set", "subject": "claude", "body": capability(&["项目/Studiolo"], propose)}).to_string(),
                ),
                200,
            )
            .await
            .json();
        assert_eq!(submitted["desired"]["status"], "validated");
        ids.push(submitted["desired"]["id"].as_str().unwrap().to_owned());
    }
    let (older, newer) = (&ids[0], &ids[1]);

    f.elevate(&login).await;
    let refused = f
        .send(
            f.request(
                "POST",
                ACTIVATE,
                Some(&login),
                &json!({"desired_id": older, "expected_effective_hash": initial}).to_string(),
            ),
            409,
        )
        .await
        .json();
    assert_eq!(refused["code"], "newer_desired_exists");
    assert_eq!(refused["newer_desired_ids"], json!([newer]));
    // 什么都没写：effective 不变，两份都还是 validated；拒绝进审计（Intent 已先落账）。
    assert_eq!(f.status(&login).await["effective_hash"], initial);
    let refusals = f.events("policy.activate_denied");
    assert_eq!(refusals.len(), 1);
    assert_eq!(
        refusals[0].event().payload().as_value()["reason"],
        "newer_desired_exists"
    );
    assert_eq!(f.events("policy.activated").len(), 1, "只有 Intent");
    let listed = f
        .send(f.request("GET", DESIRED, Some(&login), ""), 200)
        .await
        .json();
    let statuses: Vec<_> = listed["desired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["status"].clone())
        .collect();
    assert_eq!(statuses, [json!("validated"), json!("validated")]);
    assert_eq!(listed["newer_open"][older], json!([newer]));
    assert_eq!(listed["newer_open"][newer], json!([]));

    // 发布最新那份：照旧发布，较早那份标为已取代，列表里不再有未发布草案。
    let published = f
        .send(
            f.request(
                "POST",
                ACTIVATE,
                Some(&login),
                &json!({"desired_id": newer, "expected_effective_hash": initial}).to_string(),
            ),
            200,
        )
        .await
        .json();
    assert_eq!(published["activation"]["desired_id"], json!(newer));
    assert_eq!(
        f.policy().desired(older).unwrap().unwrap().status,
        DesiredStatus::Superseded
    );
    let listed = f
        .send(f.request("GET", DESIRED, Some(&login), ""), 200)
        .await
        .json();
    assert_eq!(listed["newer_open"], json!({}));
    f.stop().await;
}

#[tokio::test]
async fn revocation_is_l2_and_release_of_global_is_l3() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = |level: &str, target: Option<&str>| {
        let mut value = json!({"level": level, "reason": "测试"});
        if let Some(target) = target {
            value["target"] = json!(target);
        }
        value.to_string()
    };
    for (level, target) in [
        ("instance", Some("01INSTANCE")),
        ("type", Some("claude")),
        ("global", None),
    ] {
        let reply = f
            .send(
                f.request("POST", REVOKE, Some(&login), &body(level, target)),
                200,
            )
            .await
            .json();
        assert_eq!(reply["created"], true, "{level}");
        assert_eq!(reply["level"], level);
    }
    let again = f
        .send(
            f.request("POST", REVOKE, Some(&login), &body("global", None)),
            200,
        )
        .await
        .json();
    assert_eq!(again["created"], false);
    for (action, expected) in [
        ("agent_instance.revoked", 1),
        ("agent_type.revoked", 1),
        ("agent.global_revoked", 1),
    ] {
        assert_eq!(f.events(action).len(), expected, "{action}");
    }
    let status = f.status(&login).await;
    assert_eq!(status["global_revoked"], true);
    assert_eq!(status["revocations"].as_array().unwrap().len(), 3);
    for bad in [
        body("global", Some("x")),
        body("type", None),
        body("nope", Some("x")),
        json!({"level": "type", "target": "claude", "reason": ""}).to_string(),
        json!({"level": "type", "target": "claude", "reason": "x", "extra": 1}).to_string(),
    ] {
        f.send(f.request("POST", REVOKE, Some(&login), &bad), 400)
            .await;
    }
    // 解除全局吊销是 L3：没有近期认证 403；有则 200 且审计。
    let effective = status["effective_hash"].as_str().unwrap().to_owned();
    let release = json!({"release_global_revocation": true, "expected_effective_hash": effective})
        .to_string();
    f.send(f.request("POST", ACTIVATE, Some(&login), &release), 403)
        .await;
    f.elevate(&login).await;
    let released = f
        .send(f.request("POST", ACTIVATE, Some(&login), &release), 200)
        .await
        .json();
    assert_eq!(released["released"], true);
    assert_eq!(f.events("agent.global_released").len(), 1);
    assert_eq!(f.status(&login).await["global_revoked"], false);
    // 没有可解除的全局吊销：409。
    f.send(f.request("POST", ACTIVATE, Some(&login), &release), 409)
        .await;
    f.stop().await;
}

#[tokio::test]
async fn policy_routes_are_closed_and_absent_without_the_plane() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    for (path, allow) in [
        (STATUS, "GET"),
        (VERSIONS, "GET"),
        (VALIDATE, "POST"),
        (ACTIVATE, "POST"),
        (REVOKE, "POST"),
    ] {
        let method = if allow == "GET" { "POST" } else { "GET" };
        let reply = f.send(f.request(method, path, Some(&login), ""), 405).await;
        assert_eq!(reply.headers["allow"], allow, "{path}");
        f.send(
            f.request(allow, &format!("{path}?x=1"), Some(&login), "{}"),
            400,
        )
        .await;
    }
    f.send(f.request("PUT", DESIRED, Some(&login), "{}"), 405)
        .await;
    for bad in [
        "[]",
        "{}",
        json!({"kind": "rules", "subject": "a.md"})
            .to_string()
            .as_str(),
        json!({"kind": "rules", "subject": "a.md", "body": "# x", "extra": 1})
            .to_string()
            .as_str(),
    ] {
        f.send(f.request("POST", DESIRED, Some(&login), bad), 400)
            .await;
    }
    for bad in [
        json!({"expected_effective_hash": "0"}).to_string(),
        json!({"desired_id": "01ABCDEFGHJKMNPQRSTVWXYZ00", "rollback_to": 1, "expected_effective_hash": "0".repeat(64)}).to_string(),
        json!({"desired_id": "short", "expected_effective_hash": "0".repeat(64)}).to_string(),
    ] {
        f.send(f.request("POST", ACTIVATE, Some(&login), &bad), 400)
            .await;
    }
    f.stop().await;

    let mut bare = Fixture::new(false).await;
    let login = bare.issue().await;
    bare.send(bare.request("GET", STATUS, Some(&login), ""), 404)
        .await;
    bare.send(
        bare.request(
            "POST",
            REVOKE,
            Some(&login),
            r#"{"level":"global","reason":"x"}"#,
        ),
        404,
    )
    .await;
    bare.stop().await;
}
