use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn connection_refuses_sql_writes() {
    let directory = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let _writer = SqliteAuditLedger::open(directory.path()).unwrap();
    let reader = AuditReadView::open_existing(directory.path()).unwrap();
    let session = reader.ledger.lock().unwrap();
    assert!(session.connection.is_readonly("main").unwrap());
    assert_eq!(
        session
            .connection
            .execute("DELETE FROM audit_events", [])
            .unwrap_err()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::ReadOnly)
    );
}
