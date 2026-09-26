//! 真进程验证CLI能力开关已贯通启动装配和HTTP，所有资料均为临时虚构夹具。

#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "loopback_port/mod.rs"]
#[allow(dead_code)]
mod loopback_port;
#[path = "access/test_support.rs"]
#[allow(dead_code)]
mod test_support;

use loopback_port::{Announced, Reservation};
use std::{fs, process::Command, time::Duration};

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
        // 端口由占位持有到子进程退出，就绪以子进程自己的启动行为准（T-171）。
        let reservation = Reservation::new();
        let port = reservation.address.port();
        // 覆盖只供原生测试选取随包产品，不传入产品环境。
        let binary = std::env::var_os("STUDIOLO_TEST_SERVER_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_BIN_EXE_studiolo-local-server"))
            });
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
            &format!("{}={}", fixture::PROJECT, f.repository.path().display()),
        ]);
        if enabled {
            command.arg("--cargo-dependencies");
        }
        let process = Announced::spawn(command, reservation, Duration::from_secs(8));
        let endpoint = format!(
            "http://{}{}",
            process.address,
            fixture::uri(fixture::PROJECT, 1)
        );
        let response = client.get(&endpoint).send().await.unwrap();
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
