//! 公开时钟 Seam 让会话在预检后、最终执行观察时到期；不伪造授权 Context。

use super::{client::Client, common::*, fixture::Fixture, support::*, test_support::verifier};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{FactorRuntime, OsSessionRandom, SessionClock, SessionError, SessionManager};
use studiolo_local_server::{PreparedUnixIngress, router_with_factors};
use studiolo_vault_broker::ReadOnlyVault;

struct ExpireAfterPrecheck {
    now: i64,
    observations: Mutex<Observations>,
}

#[derive(Debug, PartialEq, Eq)]
enum Phase {
    Initialization,
    Precheck,
    Execution,
}

struct Observations {
    phase: Phase,
    initialization: usize,
    request: usize,
}

impl ExpireAfterPrecheck {
    fn begin_request(&self) {
        let mut observations = self.observations.lock().unwrap();
        assert_eq!(observations.phase, Phase::Initialization);
        assert_eq!(observations.initialization, 1);
        assert_eq!(observations.request, 0);
        observations.phase = Phase::Precheck;
    }
}

impl SessionClock for ExpireAfterPrecheck {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        let mut observations = self.observations.lock().unwrap();
        Ok(match observations.phase {
            Phase::Initialization => {
                observations.initialization += 1;
                self.now
            }
            Phase::Precheck => {
                observations.request += 1;
                observations.phase = Phase::Execution;
                self.now
            }
            Phase::Execution => {
                observations.request += 1;
                self.now + 1_800_000
            }
        })
    }
}

#[tokio::test]
async fn execution_rejection_clears_state_cookie_but_factor_action_still_returns_403() {
    for (path, expected) in [(STATE, 401), (STEP_UP, 403)] {
        let mut fixture = Fixture::new(true).await;
        let login = fixture.issue().await;
        fixture.stop().await;
        let clock = Arc::new(ExpireAfterPrecheck {
            now: fixture.now(),
            observations: Mutex::new(Observations {
                phase: Phase::Initialization,
                initialization: 0,
                request: 0,
            }),
        });
        let sessions = Arc::new(
            SessionManager::open_existing(
                &fixture.root.join("auth"),
                config(),
                Arc::new(SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap()),
                clock.clone(),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        let factors = Arc::new(
            FactorRuntime::attach(sessions.clone(), seed(false), Some(seed(true))).unwrap(),
        );
        let app = router_with_factors(
            (
                ReadOnlyVault::open(&fixture.root.join("vault")).unwrap(),
                None,
            ),
            ReadOnlyVault::open(&fixture.root.join("web")).unwrap(),
            "虚构执行夹具".into(),
            verifier().await,
            sessions,
            factors,
        )
        .unwrap();
        let isolated = ["auth", "audit", "vault", "web"].map(|name| fixture.root.join(name));
        let roots = isolated
            .iter()
            .map(|path| path.as_path())
            .collect::<Vec<_>>();
        let bound = PreparedUnixIngress::prepare(&fixture.socket, &roots)
            .unwrap()
            .bind()
            .unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(bound.serve_shutdown(app, async {
            let _ = stopped.await;
        }));
        // open_existing 经 open_mode 的 manager.run 观察一次初始化时间。
        // 装配结束后显式进入请求阶段：预检仍有效，执行观察才推进到会话到期。
        clock.begin_request();
        let input = if path == STATE {
            request(&fixture, Some(&login))
        } else {
            fixture.request(
                "POST",
                path,
                Some(&login),
                &format!("{{\"code\":\"{}\"}}", otp(&OLD, fixture.now())),
            )
        };
        let reply = Client::connect(&fixture.socket).await.send(input).await;
        assert_eq!(reply.status, expected);
        reply.secure();
        if path == STATE {
            reply.cleared_cookie();
        } else {
            reply.kept_cookie();
        }
        no_secrets(&reply, &login, &[]);
        keys(&reply.json(), &["error"]);
        assert_eq!(clock.observations.lock().unwrap().request, 2);
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(6), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
