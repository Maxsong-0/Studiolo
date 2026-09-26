//! 真进程验证显式绑定启动与失败不监听；仅使用虚构资料和自有子进程。
//! 端口由占位持有到子进程退出，就绪以子进程自己的启动行为准（T-171，见 `loopback_port`）。

#[path = "../loopback_port/mod.rs"]
#[allow(dead_code)]
mod loopback_port;

use loopback_port::{Announced, Reservation};
use std::{net::TcpListener, process::Command, time::Duration};

use super::fixture::{Fixture, PROJECT, uri};

fn command(f: &Fixture, port: u16, binding: &str) -> Command {
    // 仅测试选择随发布包验证的生产程序，不依赖构建机绝对路径。
    let binary = std::env::var_os("STUDIOLO_TEST_SERVER_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_studiolo-local-server")));
    assert!(
        binary.is_absolute() && binary.is_file(),
        "测试产品二进制必须为已存在的绝对文件路径"
    );
    let mut command = Command::new(binary);
    command.env_remove("STUDIOLO_TEST_SERVER_BIN");
    command.args([
        "--vault",
        f.vault.path().to_str().unwrap(),
        "--assets",
        f.assets.path().to_str().unwrap(),
        "--port",
        &port.to_string(),
        "--project-repository",
        binding,
    ]);
    command
}

#[tokio::test]
async fn real_binary_serves_bound_directory_evidence() {
    let f = Fixture::new();
    let reservation = Reservation::new();
    let binding = format!("{PROJECT}={}", f.repository.path().display());
    let command = command(&f, reservation.address.port(), &binding);
    let process = Announced::spawn(command, reservation, Duration::from_secs(8));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let endpoint = format!("http://{}{}", process.address, uri(PROJECT, 1));
    let response = client.get(&endpoint).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let value: serde_json::Value =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(value["repository"]["status"], "ready");
    assert_eq!(
        super::fixture::drift(&value, "src/extra").as_deref(),
        Some("undocumented")
    );
}

#[test]
fn missing_or_symbolic_binding_fails_without_creating_it_or_listening() {
    let f = Fixture::new();
    let missing = f.repository.path().join("missing-root");
    // 占位持有到本测试结束：退出后仍能在同一端口绑定监听，说明失败进程没有留下监听，
    // 而端口不会在两次启动之间被同机进程拿走、造成误判。
    let reservation = Reservation::new();
    let port = reservation.address.port();
    let binding = format!("{PROJECT}={}", missing.display());
    let output = command(&f, port, &binding).output().unwrap();
    assert!(!output.status.success());
    assert!(!missing.exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(missing.to_str().unwrap()));
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
    drop(listener);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(f.assets.path(), &missing).unwrap();
        let output = command(&f, port, &binding).output().unwrap();
        assert!(!output.status.success());
        assert!(TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).is_ok());
    }
}
