//! 完整 Auth→Audit→Auth 在途期间不能封存；会话读取的隐式写入同样受准入约束。
use super::*;
use std::{
    sync::{Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, OperationPermit,
    SqliteAuditLedger,
};

struct Paused {
    ledger: Arc<SqliteAuditLedger>,
    arrived: mpsc::SyncSender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl AuditLedger for Paused {
    fn enter_operation(&self) -> Result<OperationPermit, AuditError> {
        self.ledger.enter_operation()
    }
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let row = self.ledger.append(event.clone())?;
        if event.action() == "session.issue" && event.phase() == AuditPhase::Outcome {
            self.arrived.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        Ok(row)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[test]
fn checkpoint_waits_for_post_audit_auth_commit_and_implicit_read_writes() {
    let auth = directory();
    let audit = directory();
    let ledger = Arc::new(SqliteAuditLedger::open(audit.path()).unwrap());
    let (arrived, arrival) = mpsc::sync_channel(1);
    let (resume, release) = mpsc::sync_channel(1);
    let paused = Arc::new(Paused {
        ledger: ledger.clone(),
        arrived,
        resume: Mutex::new(release),
    });
    let clock = Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1_000_000)));
    let sessions = Arc::new(
        SessionManager::open(
            auth.path(),
            config(10, 100),
            paused,
            clock.clone(),
            Arc::new(TestRandom::default()),
        )
        .unwrap(),
    );
    let operation = {
        let sessions = sessions.clone();
        thread::spawn(move || sessions.issue(&identity()))
    };
    arrival.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        scalar(
            &sessions,
            "SELECT count(*) FROM sessions WHERE state='pending'"
        ),
        1
    );
    assert!(
        ledger
            .quiesce_operations(Instant::now() + Duration::from_millis(40))
            .is_err()
    );
    resume.send(()).unwrap();
    let issued = operation.join().unwrap().unwrap();
    let frozen = ledger
        .quiesce_operations(Instant::now() + Duration::from_secs(1))
        .unwrap();
    clock.set(1_001_000);
    let (started, start) = mpsc::sync_channel(1);
    let (finished, finish) = mpsc::sync_channel(1);
    let reader = {
        let sessions = sessions.clone();
        thread::spawn(move || {
            started.send(()).unwrap();
            let result = sessions.check(&identity(), &issued.token);
            finished.send(()).unwrap();
            result
        })
    };
    start.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(finish.recv_timeout(Duration::from_millis(40)).is_err());
    assert_eq!(
        scalar(&sessions, "SELECT last_seen FROM sessions"),
        1_000_000
    );
    drop(frozen);
    assert_eq!(reader.join().unwrap().unwrap().last_seen_at_ms, 1_001_000);
}
