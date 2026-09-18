//! 旧本机启动的真实 TCP 契约；不模拟 HTTPS 浏览器 Cookie 或生产 Access。

use super::{fixture::Fixture, process::Running};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

#[test]
fn original_local_binary_still_serves_read_only_without_auth_or_secret() {
    let fixture = Fixture::new();
    let note = fs::read(fixture.vault.join("fixture.md")).unwrap();
    let html = fs::read(fixture.web.join("index.html")).unwrap();
    let mut server = Running::start(&fixture.root, fixture.legacy_args());
    server.wait_ready();
    let page = request(server.address, "GET", "/");
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("启动夹具"));
    let snapshot = request(server.address, "GET", "/api/workbench");
    assert!(snapshot.starts_with("HTTP/1.1 200"));
    // 首屏只报计数；逐篇清单改走文件树与搜索各自的分页入口，不再整份随首屏下发。
    assert!(snapshot.contains(r#""documents":{"value":1"#));
    assert!(!snapshot.contains(r#""documents":["#));
    let script = request(server.address, "GET", "/assets/app.js");
    assert!(script.starts_with("HTTP/1.1 200"));
    assert!(script.contains("const fixture = true;"));
    assert!(request(server.address, "HEAD", "/").starts_with("HTTP/1.1 200"));
    assert!(request(server.address, "POST", "/api/workbench").starts_with("HTTP/1.1 405"));
    assert!(request(server.address, "GET", "/api/v1/session").starts_with("HTTP/1.1 404"));
    assert!(!snapshot.to_ascii_lowercase().contains("set-cookie:"));
    drop(server);
    fixture.empty_stores();
    assert_eq!(fs::read(fixture.vault.join("fixture.md")).unwrap(), note);
    assert_eq!(fs::read(fixture.web.join("index.html")).unwrap(), html);
}

#[test]
fn help_exposes_explicit_session_mode_without_opening_stores() {
    let fixture = Fixture::new();
    let output = Running::start(&fixture.root, vec!["--help".into()]).wait();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--session-config"));
    assert!(help.contains("--public-origin"));
    fixture.empty_stores();
}

fn request(address: SocketAddr, method: &str, path: &str) -> String {
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(socket, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").unwrap();
    let mut output = Vec::new();
    socket.take(64 * 1024).read_to_end(&mut output).unwrap();
    String::from_utf8(output).unwrap()
}
