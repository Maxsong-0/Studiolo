use super::*;
fn fresh() -> (tempfile::TempDir, Arc<SqliteAuditLedger>) {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let ledger = Arc::new(SqliteAuditLedger::open(root.path()).unwrap());
    (root, ledger)
}
#[test]
fn persistent_admission_is_required_and_resume_never_repairs_or_creates_it() {
    let (_root, ledger) = fresh();
    assert!(BackupAdmission::resume(ledger.clone(), &key(), TARGET).is_err());
    assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    let grant = fixture::grant(&ledger, |_| {});
    assert!(BackupAdmission::resume(ledger.clone(), &key(), TARGET).is_err());
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 1);
    let (_other_root, other) = fresh();
    other.append(grant.clone()).unwrap();
    assert!(BackupAdmission::resume(other.clone(), &key(), TARGET).is_err());
    assert_eq!(other.query(&AuditFilter::default()).unwrap().len(), 1);
    let receipt = ledger.append(grant).unwrap();
    assert!(BackupAdmission::resume(ledger.clone(), &key(), TARGET).is_ok());
    let again = ledger.query(&AuditFilter::default()).unwrap();
    assert_eq!(again.len(), 2);
    assert_eq!(again[1], receipt);
}
#[test]
fn forged_source_cutoff_schema_key_target_and_unknown_fields_cannot_authorize_a_port() {
    for (field, replacement) in [
        ("schema", json!(2)),
        ("audit_hash", json!("ff".repeat(32))),
        ("audit_offset", json!(0)),
        ("audit_offset", json!(2)),
        ("content_cutoff", json!(-1)),
        ("expires_at", json!(0)),
        (
            "source",
            json!({"checkpoint_id":"other","seal_sha256":"ab".repeat(32)}),
        ),
        (
            "source",
            json!({"checkpoint_id":"approved-job","seal_sha256":"ab"}),
        ),
        ("key", json!({"sequence":2,"checkpoint_id":"approved-job"})),
        ("target", json!(BackupTargetKind::LocalTemporaryRepository)),
        ("unexpected", json!(true)),
    ] {
        let (_root, ledger) = fresh();
        let grant = fixture::grant(&ledger, |body| body[field] = replacement);
        ledger.append(grant).unwrap();
        assert!(
            BackupAdmission::resume(ledger.clone(), &key(), TARGET).is_err(),
            "{field}"
        );
        assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
    }
}
#[test]
fn expired_source_keeps_original_audit_authority_but_cannot_be_delivered_again() {
    let (_root, ledger) = fresh();
    let grant = fixture::grant(&ledger, |body| body["expires_at"] = json!(1));
    ledger.append(grant).unwrap();
    let admission = BackupAdmission::resume(ledger.clone(), &key(), TARGET).unwrap();
    assert!(admission.check_live().is_err());
    assert_eq!(admission.source().seal_sha256, "ab".repeat(32));
    assert_eq!(admission.expires_at().millis(), 1);
    let port = BackupAuditPort::new(admission);
    assert!(
        port.dispatch(&failed("upload_rejected").encode().unwrap())
            .is_ok()
    );
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 3);
}
#[test]
fn worker_protocol_cannot_create_admissions_even_with_the_original_job() {
    let (_root, ledger) = fresh();
    let grant = fixture::grant(&ledger, |_| {});
    assert!(BackupAuditRequest::append(key(), TARGET, grant).is_err());
    assert!(BackupAdmission::resume(ledger.clone(), &key(), TARGET).is_err());
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 1);
}
