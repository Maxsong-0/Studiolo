//! 在真实Intent持久操作期间撤权/漂移；最终授权前的大对象准备不能变成可见执行。
#[path = "file_operation_pause.rs"]
mod pause;
use crate::{
    file_operations::{delete, post},
    fixture::Fixture,
    support,
};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_after_prepare_cancels_with_both_real_acks_before_401() {
    let audit = Arc::new(pause::Paused::default());
    let f = Arc::new(Fixture::file_operations(Some(audit.clone())).await);
    let issued = f.http.issue().await;
    let cookie = issued.cookie.clone();
    let csrf = issued.csrf.clone();
    let job = {
        let f = f.clone();
        tokio::spawn(
            async move { post(&f, &issued, &delete(&f, "revoke-during-prepare"), 401).await },
        )
    };
    let observed = audit.clone();
    tokio::task::spawn_blocking(move || observed.wait())
        .await
        .unwrap();
    let mut revoke = f.http.request(
        "POST",
        support::REVOKE,
        Some(&cookie),
        r#"{"scope":"current"}"#,
    );
    revoke
        .headers_mut()
        .insert("x-csrf-token", csrf.parse().unwrap());
    f.http.send(revoke, 200).await;
    audit.release();
    assert_eq!(job.await.unwrap()["code"], "session_required");
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert!(f.journal()["file_operation"].is_null());
    let records = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[1].event().payload().as_value()["file_operation"]["outcome"],
        "cancelled"
    );
    match Arc::try_unwrap(f) {
        Ok(f) => f.close().await,
        Err(_) => panic!("夹具仍被持有"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_file_drift_after_prepare_is_cancelled_without_overwriting_external_bytes() {
    let audit = Arc::new(pause::Paused::default());
    let f = Arc::new(Fixture::file_operations(Some(audit.clone())).await);
    let issued = f.http.issue().await;
    let job = {
        let f = f.clone();
        tokio::spawn(
            async move { post(&f, &issued, &delete(&f, "drift-after-prepared"), 503).await },
        )
    };
    let observed = audit.clone();
    tokio::task::spawn_blocking(move || observed.wait())
        .await
        .unwrap();
    fs::write(
        f.http.root.join("vault/a.md"),
        b"external after preparation",
    )
    .unwrap();
    audit.release();
    assert_eq!(job.await.unwrap()["code"], "stale");
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        b"external after preparation"
    );
    assert!(f.journal()["file_operation"].is_null());
    let records = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(
        records[1].event().payload().as_value()["file_operation"]["outcome"],
        "cancelled"
    );
    match Arc::try_unwrap(f) {
        Ok(f) => f.close().await,
        Err(_) => panic!("夹具仍被持有"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_config_drift_cannot_discard_a_persisted_request_as_an_ordinary_denial() {
    let audit = Arc::new(pause::Paused::default());
    let f = Arc::new(Fixture::file_operations(Some(audit.clone())).await);
    let issued = f.http.issue().await;
    let job = {
        let f = f.clone();
        tokio::spawn(
            async move { post(&f, &issued, &delete(&f, "config-after-prepare"), 503).await },
        )
    };
    let observed = audit.clone();
    tokio::task::spawn_blocking(move || observed.wait())
        .await
        .unwrap();
    let config = f.http.root.join("config/owner.json");
    let mut bytes = fs::read(&config).unwrap();
    bytes.push(b' ');
    fs::write(&config, bytes).unwrap();
    audit.release();
    assert_eq!(job.await.unwrap()["code"], "outcome_unknown");
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert!(!f.journal()["file_operation"].is_null());
    match Arc::try_unwrap(f) {
        Ok(f) => f.close().await,
        Err(_) => panic!("夹具仍被持有"),
    }
}
