//! 真进程验证CLI能力开关已贯通启动装配和HTTP，所有资料均为临时虚构夹具。

#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "access/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use std::{
    fs,
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn real_binary_only_reads_manifests_with_explicit_cargo_opt_in() {
    let f = fixture::Fixture::new();
    fs::write(
        f.repository.path().join("Cargo.toml"),
        "[package]\nname = 'fictional-package'\nversion = '0.0.1'\n\
         [package.metadata]\nprivate = 'MANIFEST_PRIVATE_MUST_NOT_ESCAPE'\n",
    )
    .unwrap();
    fs::write(
        f.repository.path().join("build.rs"),
        "THIS_MUST_NEVER_BE_EXECUTED",
    )
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    for enabled in [false, true] {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut command = Command::new(env!("CARGO_BIN_EXE_studiolo-local-server"));
        command.args([
            "--vault",
            f.vault.path().to_str().unwrap(),
            "--assets",
            f.assets.path().to_str().unwrap(),
            "--port",
            &port.to_string(),
            "--project-repository",
            &format!("{}={}", fixture::PROJECT, f.repository.path().display()),
        ]);
        if enabled {
            command.arg("--cargo-dependencies");
        }
        let _process = Running(
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let endpoint = format!(
            "http://127.0.0.1:{port}{}",
            fixture::uri(fixture::PROJECT, 1)
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        let response = loop {
            if let Ok(response) = client.get(&endpoint).send().await {
                break response;
            }
            assert!(Instant::now() < deadline, "测试子进程未及时监听");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(response.status(), 200);
        let body = response.bytes().await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let cargo = &value["cargo_dependencies"];
        assert_eq!(cargo["status"], if enabled { "ready" } else { "disabled" });
        if enabled {
            assert_eq!(cargo["graph"]["packages"][0]["name"], "fictional-package");
        } else {
            assert!(cargo["graph"].is_null());
        }
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(!text.contains("MANIFEST_PRIVATE_MUST_NOT_ESCAPE"));
        assert!(!text.contains("THIS_MUST_NEVER_BE_EXECUTED"));
    }
}
