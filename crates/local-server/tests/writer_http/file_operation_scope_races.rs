//! 最终真实 Auth 核验等待时，私有批准漂移不能安装，也不能假报安全取消。
#[path = "file_operation_scope_startup.rs"]
mod startup;
#[path = "file_operation_scope_support.rs"]
mod support;
use crate::{
    file_operations::{delete, post},
    fixture::Fixture,
    support as http_support, test_support,
};
use serde_json::Value;
use std::{
    fs,
    sync::{Arc, atomic::AtomicBool},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};
use studiolo_auth::{OsSessionRandom, SessionManager};
use studiolo_vault_broker::ReadOnlyVault;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_drift_inside_final_session_verify_retains_unacknowledged_cancellation() {
    let armed = Arc::new(AtomicBool::new(false));
    let audit = Arc::new(support::IntentAudit::new(armed.clone()));
    let mut f = Fixture::file_operations(Some(audit.clone())).await;
    let clock = Arc::new(support::FinalClock::new(f.http.now(), armed));
    // 从原真实 Auth 存储重开，唯一变化是可信装配的时钟 adapter。
    let sessions = Arc::new(
        SessionManager::open_existing(
            &f.http.auth,
            http_support::config(),
            Arc::new(SqliteAuditLedger::open_existing(&f.http.audit).unwrap()),
            clock.clone(),
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    f.http.app = studiolo_local_server::router_with_canonical_writer(
        f.owner.clone(),
        ReadOnlyVault::open(&f.http.root.join("web")).unwrap(),
        "最终授权竞态夹具".into(),
        4319,
        test_support::verifier().await,
        sessions,
        None,
    )
    .unwrap();
    let f = Arc::new(f);
    let issued = f.http.issue().await;
    let job = {
        let f = f.clone();
        tokio::spawn(
            async move { post(&f, &issued, &delete(&f, "config-final-verify-01"), 503).await },
        )
    };
    let observed = clock.clone();
    tokio::task::spawn_blocking(move || observed.pause.wait())
        .await
        .unwrap();
    assert_eq!(
        clock.calls(),
        2,
        "必须暂停最终 verify，而不是此前的 file_access"
    );
    let prepared = f.journal();
    assert_eq!(prepared["file_operation"]["phase"], "prepared");
    let path = f.http.root.join("config/owner.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config.as_object_mut().unwrap().remove("file_operations");
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    clock.pause.release();
    let response = job.await.unwrap();
    assert_eq!(response["code"], "outcome_unknown");
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    let after = f.journal();
    let pending = &after["file_operation"];
    // 原 Intent 已有 ACK，因此仍可固定不执行的取消；配置漂移阻止 Outcome ACK，槽不能释放。
    assert_eq!(pending["phase"], "finalized_await_audit");
    for field in ["frozen", "before", "history", "sequence"] {
        assert_eq!(pending[field], prepared["file_operation"][field]);
    }
    assert_eq!(pending["terminal"]["receipt"]["outcome"], "cancelled");
    assert_eq!(
        pending["terminal"]["receipt"]["effects"],
        serde_json::json!([{"kind":"not_applied","path":"a.md"}])
    );
    assert!(pending["executed"].is_null() && pending["quarantine"].is_null());
    assert_eq!(after["committed_through"], 0);
    let records = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 1, "配置失效不能补出取消或成功 Outcome");
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    match Arc::try_unwrap(f) {
        Ok(f) => f.close().await,
        Err(_) => panic!("夹具仍被持有"),
    }
}
