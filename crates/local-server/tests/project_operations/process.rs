//! 真进程验证显式绑定启动与失败不监听；仅使用虚构资料和自有子进程。

use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::Duration,
};

use super::fixture::{Fixture, PROJECT, uri};

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn command(f: &Fixture, port: u16, binding: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_studiolo-local-server"));
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
    let port = port();
    let binding = format!("{PROJECT}={}", f.repository.path().display());
    let _process = Running(
        command(&f, port, &binding)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let endpoint = format!("http://127.0.0.1:{port}{}", uri(PROJECT, 1));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let response = loop {
        if let Ok(response) = client.get(&endpoint).send().await {
            break response;
        }
        assert!(std::time::Instant::now() < deadline, "测试子进程未及时监听");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
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
    let port = port();
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
