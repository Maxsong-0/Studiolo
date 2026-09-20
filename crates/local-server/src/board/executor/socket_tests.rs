use super::*;
#[test]
fn later_preparation_failure_drops_the_created_socket() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runner.sock");
    let prepare = || -> Result<(), String> {
        let (_listener, _lease) = SocketLease::bind(path.clone())?;
        Err("later database open failed".into())
    };
    assert!(prepare().is_err());
    assert!(!path.exists());
    assert!(SocketLease::bind(path).is_ok());
}
#[test]
fn cleanup_keeps_a_replacement_socket() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runner.sock");
    let (_listener, lease) = SocketLease::bind(path.clone()).unwrap();
    std::fs::remove_file(&path).unwrap();
    let _replacement = UnixListener::bind(&path).unwrap();
    drop(lease);
    assert!(path.exists());
}
#[test]
fn parent_replacement_cannot_redirect_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("original");
    let moved = directory.path().join("moved");
    std::fs::create_dir(&parent).unwrap();
    let path = parent.join("runner.sock");
    let (_listener, lease) = SocketLease::bind(path.clone()).unwrap();
    std::fs::rename(&parent, &moved).unwrap();
    std::fs::create_dir(&parent).unwrap();
    let _replacement = UnixListener::bind(&path).unwrap();
    drop(lease);
    assert!(!moved.join("runner.sock").exists());
    assert!(path.exists());
}

#[test]
fn explicit_cleanup_retires_the_lease_before_a_later_drop() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runner.sock");
    let (listener, lease) = SocketLease::bind(path.clone()).unwrap();
    drop(listener);
    lease.cleanup().unwrap();
    let _replacement = UnixListener::bind(&path).unwrap();
    drop(lease);
    assert!(path.exists());
}
