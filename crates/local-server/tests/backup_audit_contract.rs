//! 真实 SQLite 账本的单作业端口；传输鉴权另验，不用本测试冒充双 UID 接线。
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    num::NonZeroU64,
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger,
};
use studiolo_backup::worker::{
    audit::BackupAuditRequest, job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};
use studiolo_core::checkpoint::CheckpointId;
use studiolo_local_server::backup::admission::BackupAdmission;
use studiolo_local_server::backup::audit::BackupAuditPort;
#[path = "backup_audit/admission.rs"]
mod admission;
#[path = "backup_audit/fixture.rs"]
mod fixture;
const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
fn key() -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("approved-job").unwrap(),
    }
}
fn setup() -> (tempfile::TempDir, Arc<SqliteAuditLedger>, BackupAuditPort) {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = Arc::new(SqliteAuditLedger::open(root.path()).unwrap());
    let port = BackupAuditPort::new(fixture::existing(&ledger));
    (root, ledger, port)
}
fn intent() -> BackupAuditRequest {
    BackupAuditRequest::append(key(), TARGET, backup_intent(&key(), TARGET).unwrap()).unwrap()
}
fn failed(reason: &str) -> BackupAuditRequest {
    let mut event = serde_json::to_value(backup_intent(&key(), TARGET).unwrap()).unwrap();
    event["phase"] = json!("outcome");
    event["payload"] = json!({"job_sequence":1,"checkpoint_id":"approved-job","result":"failed","reason":reason,"recorded_after_checkpoint_cutoff":true});
    BackupAuditRequest::append(
        key(),
        TARGET,
        serde_json::from_value::<AuditEvent>(event).unwrap(),
    )
    .unwrap()
}
#[test]
fn original_intent_and_outcome_are_durable_idempotent_and_readable_only_for_this_job() {
    let (root, ledger, port) = setup();
    let request = intent();
    let bytes = request.encode().unwrap();
    let first = port.dispatch(&bytes).unwrap();
    assert_eq!(first, port.dispatch(&bytes).unwrap());
    let original = request.decode_reply(&first).unwrap().unwrap();
    assert_eq!(original.event(), &backup_intent(&key(), TARGET).unwrap());
    let read = BackupAuditRequest::read(key(), TARGET, AuditPhase::Intent);
    assert_eq!(
        read.decode_reply(&port.dispatch(&read.encode().unwrap()).unwrap())
            .unwrap(),
        Some(original)
    );
    let failure = failed("upload_rejected");
    let outcome = port.dispatch(&failure.encode().unwrap()).unwrap();
    assert_eq!(outcome, port.dispatch(&failure.encode().unwrap()).unwrap());
    assert!(matches!(
        port.dispatch(&failed("out_of_space").encode().unwrap()),
        Err(AuditError::IdempotencyConflict { .. })
    ));
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 3);
    drop((port, ledger));
    let reopened = Arc::new(SqliteAuditLedger::open_existing(root.path()).unwrap());
    let admission = BackupAdmission::resume(reopened, &key(), TARGET).unwrap();
    let port = BackupAuditPort::new(admission);
    assert_eq!(outcome, port.dispatch(&failure.encode().unwrap()).unwrap());
}
#[test]
fn no_cross_job_queries_or_new_target_after_persistent_admission() {
    let (_root, ledger, port) = setup();
    let lookup = BackupAuditRequest::read(key(), TARGET, AuditPhase::Outcome);
    assert!(
        lookup
            .decode_reply(&port.dispatch(&lookup.encode().unwrap()).unwrap())
            .unwrap()
            .is_none()
    );
    let mut other = key();
    other.sequence = NonZeroU64::new(2).unwrap();
    for request in [
        BackupAuditRequest::read(other.clone(), TARGET, AuditPhase::Intent),
        BackupAuditRequest::append(
            other.clone(),
            TARGET,
            backup_intent(&other, TARGET).unwrap(),
        )
        .unwrap(),
        BackupAuditRequest::read(
            key(),
            BackupTargetKind::LocalTemporaryRepository,
            AuditPhase::Intent,
        ),
    ] {
        assert!(port.dispatch(&request.encode().unwrap()).is_err());
    }
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}
#[test]
fn invalid_message_and_replaced_ledger_do_not_initialize_or_write_another_store() {
    let (root, ledger, port) = setup();
    assert!(port.dispatch(b"{}").is_err());
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
    let original = root.path().join("audit.sqlite3");
    fs::rename(&original, root.path().join("original.sqlite3")).unwrap();
    assert!(port.dispatch(&intent().encode().unwrap()).is_err());
    assert!(!original.exists());
}
#[test]
fn dispatch_participates_in_the_original_complete_operation_gate() {
    let (_root, ledger, port) = setup();
    let quiet = ledger
        .quiesce_operations(Instant::now() + Duration::from_secs(2))
        .unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(port.dispatch(&intent().encode().unwrap()))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
    drop(quiet);
    assert!(
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_ok()
    );
    worker.join().unwrap();
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn production_connection_rejects_shared_uid_and_wrong_actual_peer_before_dispatch() {
    use std::os::unix::net::UnixStream;
    use studiolo_backup::worker::audit::transport::AuditTransportError;
    let (_root, ledger, port) = setup();
    let own = rustix::process::geteuid().as_raw();
    for expected in [own, own + 1, 0, u32::MAX] {
        let (_a, b) = UnixStream::pair().unwrap();
        assert_eq!(
            port.serve_connection(b, expected, Instant::now() + Duration::from_secs(1)),
            Err(AuditTransportError::Peer)
        );
    }
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}

#[cfg(target_os = "linux")]
#[test]
fn real_socket_ack_loss_replays_the_original_sqlite_record_without_duplicate_event() {
    use std::os::unix::net::UnixStream;
    use studiolo_backup::worker::audit::transport::AuditSocket;
    let (_root, ledger, port) = setup();
    let port = Arc::new(port);
    let uid = rustix::process::geteuid().as_raw();
    // 同 UID 仅验证真实传输与持久回执交错；生产入口拒绝同 UID，见上一用例。
    for lose_reply in [true, false] {
        let (a, b) = UnixStream::pair().unwrap();
        let port = port.clone();
        let server = std::thread::spawn(move || {
            let mut socket =
                AuditSocket::new(b, uid, Instant::now() + Duration::from_secs(2)).unwrap();
            let request = socket.receive().unwrap();
            let response = port.dispatch(&request).unwrap();
            if !lose_reply {
                socket.send(&response).unwrap();
            }
        });
        let mut socket = AuditSocket::new(a, uid, Instant::now() + Duration::from_secs(2)).unwrap();
        let request = failed("upload_rejected");
        socket.send(&request.encode().unwrap()).unwrap();
        let response = socket.receive();
        if lose_reply {
            assert!(response.is_err());
        } else {
            let record = request.decode_reply(&response.unwrap()).unwrap().unwrap();
            assert_eq!(record.offset(), 3);
        }
        server.join().unwrap();
        assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 3);
    }
}
