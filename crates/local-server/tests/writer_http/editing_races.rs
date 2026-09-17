//! 覆盖真实 HTTP 的缺失 ABA、生成区基线和安装后审计阻塞窗口。
use crate::{
    editing::{body, refresh, save},
    fixture::Fixture,
};
use std::{
    fs,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

#[tokio::test]
async fn missing_rejection_survives_recreating_the_same_baseline() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let input = body(&f, "本地草稿", "missing-aba-save-001");
    let path = f.http.root.join("vault/a.md");
    fs::remove_file(&path).unwrap();
    refresh(&f, &issued).await;
    let rejected = save(&f, &issued, &input, 409).await;
    assert!(rejected["actual_revision"].is_null());
    fs::write(&path, &f.expected.content).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(save(&f, &issued, &input, 409).await, rejected);
    assert_eq!(fs::read(path).unwrap(), f.expected.content);
    assert_eq!(f.journal()["committed_through"], 0);
    f.close().await;
}

#[tokio::test]
async fn generated_region_is_checked_against_the_exact_expected_baseline() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let first = f
        .get("/api/v1/file?path=mixed.md", &issued.cookie, 200)
        .await;
    let before = first["content"].as_str().unwrap();
    let current = before.replace("机器区", "新的机器区");
    let path = f.http.root.join("vault/mixed.md");
    fs::write(&path, &current).unwrap();
    refresh(&f, &issued).await;
    let mut input = body(
        &f,
        &current.replace("正文", "本地正文"),
        "generated-aba-save",
    );
    input["path"] = "mixed.md".into();
    input["expected_revision"] = first["revision"].clone();
    let rejected = save(&f, &issued, &input, 409).await;
    assert_eq!(rejected["code"], "revision_conflict");
    fs::write(&path, before).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(save(&f, &issued, &input, 409).await, rejected);
    assert_eq!(fs::read_to_string(path).unwrap(), before);
    f.close().await;
}

#[tokio::test]
async fn successful_replay_remains_historical_after_a_later_deletion() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let input = body(&f, "已保存", "historical-deleted-save");
    let receipt = save(&f, &issued, &input, 200).await;
    let path = f.http.root.join("vault/a.md");
    fs::remove_file(&path).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(save(&f, &issued, &input, 200).await, receipt);
    assert!(!path.exists());
    assert_eq!(f.journal()["committed_through"], 1);
    f.close().await;
}

#[derive(Default)]
struct PausedAudit {
    inner: MemoryAuditLedger,
    state: Mutex<(bool, bool)>,
    gate: Condvar,
}
impl AuditLedger for PausedAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Outcome {
            let mut state = self.state.lock().unwrap();
            state.0 = true;
            self.gate.notify_all();
            let waited = self
                .gate
                .wait_timeout_while(state, Duration::from_secs(10), |state| !state.1)
                .unwrap();
            assert!(!waited.1.timed_out(), "验收测试没有解除审计暂停");
        }
        self.inner.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_generation_is_stale_even_while_outcome_audit_has_not_finished() {
    let audit = Arc::new(PausedAudit::default());
    let f = Arc::new(Fixture::with_audit(audit.clone()).await);
    let issued = f.http.issue().await;
    let cookie = issued.cookie.clone();
    let worker = {
        let f = f.clone();
        tokio::spawn(async move {
            save(
                &f,
                &issued,
                &body(&f, "已安装但审计仍暂停", "audit-paused-save"),
                200,
            )
            .await
        })
    };
    let reached = audit.clone();
    tokio::task::spawn_blocking(move || {
        let state = reached.state.lock().unwrap();
        let waited = reached
            .gate
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.0)
            .unwrap();
        assert!(!waited.1.timed_out());
    })
    .await
    .unwrap();
    let view = f.get("/api/workbench", &cookie, 200).await;
    audit.state.lock().unwrap().1 = true;
    audit.gate.notify_all();
    worker.await.unwrap();
    assert_eq!(view["freshness"]["stale"], true);
    assert_eq!(
        fs::read_to_string(f.http.root.join("vault/a.md")).unwrap(),
        "已安装但审计仍暂停"
    );
    match Arc::try_unwrap(f) {
        Ok(f) => f.close().await,
        Err(_) => panic!("夹具仍被持有"),
    }
}
