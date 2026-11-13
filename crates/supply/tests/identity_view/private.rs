use super::*;
#[test]
fn sqlite_handle_itself_rejects_writes() {
    let dir = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let writer = AgentStore::open(dir.path()).unwrap();
    writer.issue_type("claude", "2026-11-13T00:00:00Z").unwrap();
    let view = AgentIdentityView::open_existing(dir.path()).unwrap();
    let db = view.store.lock().unwrap();
    assert!(db.is_readonly("main").unwrap());
    let error = db.execute("DELETE FROM agent_type", []).unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ReadOnly)
    );
}
