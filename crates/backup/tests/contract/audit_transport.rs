//! 真实 AF_UNIX 帧、内核 peer、限额与总截止；同 UID 夹具不冒充独立生产身份验收。
use std::{
    io::Write,
    net::Shutdown,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};
use studiolo_backup::worker::audit::{
    AUDIT_MESSAGE_MAX_BYTES,
    transport::{AuditSocket, AuditTransportError as Error},
};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}
fn peer() -> u32 {
    rustix::process::geteuid().as_raw()
}
#[test]
fn actual_socket_pair_delivers_one_half_closed_frame_in_each_direction() {
    let (a, b) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        let mut socket = AuditSocket::new(b, peer(), deadline()).unwrap();
        assert_eq!(socket.receive().unwrap(), b"request");
        socket.send(b"reply").unwrap();
        assert_eq!(socket.receive(), Err(Error::Frame));
        assert_eq!(socket.send(b"second"), Err(Error::Frame));
    });
    let mut socket = AuditSocket::new(a, peer(), deadline()).unwrap();
    socket.send(b"request").unwrap();
    assert_eq!(socket.receive().unwrap(), b"reply");
    server.join().unwrap();
}
#[test]
fn different_kernel_peer_and_expired_deadline_fail_before_io() {
    for uid in [0, u32::MAX, peer() + 1] {
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(matches!(
            AuditSocket::new(a, uid, deadline()),
            Err(Error::Peer)
        ));
    }
    let (a, _b) = UnixStream::pair().unwrap();
    assert!(matches!(
        AuditSocket::new(a, peer(), Instant::now()),
        Err(Error::Deadline)
    ));
}
#[test]
fn oversized_zero_truncated_and_pipelined_frames_are_rejected_and_poison_the_channel() {
    for bytes in [
        vec![0, 0, 0, 0],
        ((AUDIT_MESSAGE_MAX_BYTES + 1) as u32)
            .to_be_bytes()
            .to_vec(),
        vec![0, 0, 0, 3, b'a'],
        vec![0, 0, 0, 1, b'a', b'b'],
        vec![0, 0, 0],
    ] {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(&bytes).unwrap();
        a.shutdown(Shutdown::Write).unwrap();
        let mut socket = AuditSocket::new(b, peer(), deadline()).unwrap();
        assert_eq!(socket.receive(), Err(Error::Frame));
        assert_eq!(socket.send(b"cannot-reuse"), Err(Error::Frame));
    }
}
#[test]
fn absent_half_close_does_not_extend_the_original_deadline() {
    let (mut a, b) = UnixStream::pair().unwrap();
    a.write_all(&[0, 0, 0, 1, b'a']).unwrap();
    let end = Instant::now() + Duration::from_millis(70);
    let mut socket = AuditSocket::new(b, peer(), end).unwrap();
    assert_eq!(socket.receive(), Err(Error::Deadline));
    assert!(Instant::now() < end + Duration::from_secs(1));
    assert_eq!(socket.send(b"cannot-reuse"), Err(Error::Frame));
}
#[test]
fn short_reads_cannot_reset_the_shared_total_deadline() {
    let (mut a, b) = UnixStream::pair().unwrap();
    let end = Instant::now() + Duration::from_millis(90);
    let slow = std::thread::spawn(move || {
        for byte in [0, 0, 0, 8, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h'] {
            if a.write_all(&[byte]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let mut socket = AuditSocket::new(b, peer(), end).unwrap();
    assert_eq!(socket.receive(), Err(Error::Deadline));
    assert!(Instant::now() < end + Duration::from_secs(1));
    drop(socket);
    slow.join().unwrap();
}
