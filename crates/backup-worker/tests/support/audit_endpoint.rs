use super::*;
use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    time::Duration,
};
#[test]
fn actual_nonblocking_connection_checks_peer_and_exchanges_bounded_frames() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("audit.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o660)).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    let end = Instant::now() + Duration::from_secs(2);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = AuditSocket::new(stream, uid, end).unwrap();
        assert_eq!(socket.receive().unwrap(), b"bounded-request");
        socket.send(b"bounded-reply").unwrap();
    });
    // 只验证私有 connect 原语；完整生产路径的 /root 逐级约束另由 Proof 测试覆盖。
    let mut client = connect(&path, uid, end).unwrap();
    client.send(b"bounded-request").unwrap();
    assert_eq!(client.receive().unwrap(), b"bounded-reply");
    server.join().unwrap();
}
#[test]
fn wrong_peer_missing_socket_and_expired_connect_fail_without_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("audit.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    let uid = rustix::process::geteuid().as_raw();
    assert!(connect(&path, uid + 1, end).is_err());
    assert!(connect(&path, uid, Instant::now()).is_err());
    assert!(connect(&root.path().join("missing.sock"), uid, end).is_err());
}
