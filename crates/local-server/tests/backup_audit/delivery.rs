use super::*;
use studiolo_backup::worker::audit::AdmissionFact;

#[test]
fn source_delivery_is_the_original_durable_receipt_without_changing_state_or_expiry() {
    let (_root, ledger, port) = setup();
    let before = ledger.query(&AuditFilter::default()).unwrap();
    let request = BackupAuditRequest::admission(key(), TARGET);
    let bytes = request.encode().unwrap();
    let parsed = BackupAuditRequest::decode(&bytes).unwrap();
    assert!(parsed.is_admission());
    assert!(parsed.event().is_none());
    assert!(parsed.encode_reply(None).is_err());
    let reply = port.dispatch(&bytes).unwrap();
    let receipt = request.decode_reply(&reply).unwrap().unwrap();
    assert_eq!(receipt, before[1]);
    let fact = AdmissionFact::from_receipt(&receipt).unwrap();
    assert_eq!(fact.key(), &key());
    assert_eq!(fact.target(), TARGET);
    assert_eq!(fact.source().seal_sha256, "ab".repeat(32));
    assert_eq!(fact.expires_at().millis(), i64::MAX);
    assert_eq!(reply, port.dispatch(&bytes).unwrap());
    assert_eq!(before, ledger.query(&AuditFilter::default()).unwrap());
}
#[test]
fn source_delivery_rejects_other_jobs_targets_normal_intents_and_damaged_receipts() {
    let (_root, ledger, port) = setup();
    let request = BackupAuditRequest::admission(key(), TARGET);
    let reply = port.dispatch(&request.encode().unwrap()).unwrap();
    let mut other = key();
    other.sequence = NonZeroU64::new(2).unwrap();
    for request in [
        BackupAuditRequest::admission(other, TARGET),
        BackupAuditRequest::admission(key(), BackupTargetKind::LocalTemporaryRepository),
        BackupAuditRequest::read(key(), TARGET, AuditPhase::Intent),
    ] {
        assert!(request.decode_reply(&reply).is_err());
    }
    let original = ledger.query(&AuditFilter::default()).unwrap();
    assert!(request.encode_reply(Some(original[0].clone())).is_err());
    let mut malformed: serde_json::Value = serde_json::from_slice(&reply).unwrap();
    malformed["record"]["event"]["payload"]["expires_at"] = json!(7);
    assert!(
        request
            .decode_reply(&serde_json::to_vec(&malformed).unwrap())
            .is_err()
    );
    let mut bytes = request.encode().unwrap();
    bytes.push(b' ');
    assert!(BackupAuditRequest::decode(&bytes).is_err());
    assert_eq!(original, ledger.query(&AuditFilter::default()).unwrap());
}
#[cfg(target_os = "linux")]
#[test]
fn original_admission_round_trips_over_actual_bounded_unix_frames() {
    use std::os::unix::net::UnixStream;
    use studiolo_backup::worker::audit::transport::AuditSocket;
    let (_root, ledger, port) = setup();
    let uid = rustix::process::geteuid().as_raw();
    let (a, b) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        let mut socket = AuditSocket::new(b, uid, Instant::now() + Duration::from_secs(2)).unwrap();
        let request = socket.receive().unwrap();
        socket.send(&port.dispatch(&request).unwrap()).unwrap();
    });
    let request = BackupAuditRequest::admission(key(), TARGET);
    let mut socket = AuditSocket::new(a, uid, Instant::now() + Duration::from_secs(2)).unwrap();
    socket.send(&request.encode().unwrap()).unwrap();
    let receipt = request
        .decode_reply(&socket.receive().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(AdmissionFact::from_receipt(&receipt).unwrap().key(), &key());
    server.join().unwrap();
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}
