//! 在真实审计Outcome落盘时由另一公开Auth实例撤权，证明HTTP二次校验不会误清Cookie。

use super::{
    fixture::Fixture,
    support::*,
    test_support::{claims, signed, verifier},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_auth::{AccessIdentity, CsrfToken, SessionManager, SessionToken};

struct RevokeOnOutcome {
    ledger: SqliteAuditLedger,
    sessions: Arc<SessionManager>,
    identity: AccessIdentity,
    token: SessionToken,
    csrf: CsrfToken,
    fired: AtomicBool,
}

impl AuditLedger for RevokeOnOutcome {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let revoke = event.action() == "totp.step_up" && event.phase() == AuditPhase::Outcome;
        let record = self.ledger.append(event)?;
        if revoke && !self.fired.swap(true, Ordering::SeqCst) {
            self.sessions
                .revoke_all(&self.identity, &self.token, &self.csrf)
                .unwrap();
        }
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[tokio::test]
async fn revocation_after_precheck_is_forbidden_and_only_followup_check_clears_cookie() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    f.stop().await;
    let (_, cookie) = login.cookie.split_once('=').unwrap();
    let (token, _) = cookie.split_once('~').unwrap();
    let hook = Arc::new(RevokeOnOutcome {
        ledger: SqliteAuditLedger::open_existing(&f.root.join("audit")).unwrap(),
        sessions: f.manager(),
        identity: verifier().await.verify(&signed(&claims())).await.unwrap(),
        token: SessionToken::from_hex(token.to_owned()).unwrap(),
        csrf: CsrfToken::from_hex(login.csrf.clone()).unwrap(),
        fired: AtomicBool::new(false),
    });
    f.audit_override = Some(hook.clone());
    f.restart().await;
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    f.send(f.request("POST", STEP_UP, Some(&login), &body), 403)
        .await
        .kept_cookie();
    assert!(hook.fired.load(Ordering::SeqCst));
    f.send(f.request("GET", OWN, Some(&login), ""), 401)
        .await
        .cleared_cookie();
    f.stop().await;
}
