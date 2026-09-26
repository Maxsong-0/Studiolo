//! 真进程覆盖显式源码权限、异常输入后恢复，以及不执行源码/宏/构建脚本。

#[path = "project_operations/fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "loopback_port/mod.rs"]
#[allow(dead_code)]
mod loopback_port;

use loopback_port::{Announced, Reservation};
use std::{fs, process::Command, time::Duration};

use serde_json::Value;

/// 启动并等子进程自己宣布就绪；端口由占位持有到子进程退出（T-171）。
fn start(f: &fixture::Fixture, cargo: bool, rust: bool) -> (Announced, String) {
    let reservation = Reservation::new();
    let port = reservation.address.port();
    // 覆盖只供原生测试选取随包产品，不传入产品环境。
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
        &format!("{}={}", fixture::PROJECT, f.repository.path().display()),
    ]);
    if cargo {
        command.arg("--cargo-dependencies");
    }
    if rust {
        command.args(["--rust-call-sites", "[\"src/source.rs\"]"]);
    }
    let process = Announced::spawn(command, reservation, Duration::from_secs(12));
    let url = format!(
        "http://{}{}",
        process.address,
        fixture::uri(fixture::PROJECT, 1)
    );
    (process, url)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
}

/// 子进程已宣布就绪，传输错误即真实故障，不再重试掩盖。
async fn get(client: &reqwest::Client, url: &str) -> Value {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = response.bytes().await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn real_cli_keeps_cargo_and_source_permissions_independent() {
    let f = fixture::Fixture::new();
    fs::write(
        f.repository.path().join("Cargo.toml"),
        "[package]\nname = 'fictional-package'\nversion = '0.0.1'\n",
    )
    .unwrap();
    fs::write(
        f.repository.path().join("build.rs"),
        "BUILD_SCRIPT_MUST_NOT_EXECUTE",
    )
    .unwrap();
    let marker = f.repository.path().join("must-not-exist.txt");
    fs::write(
        f.repository.path().join("src/source.rs"),
        format!(
            "mod ignored; fn probe() {{ std::fs::write({:?}, \"PRIVATE_BODY\");\n\
         println!(\"MACRO_ARGUMENT\", hidden_call()); include!(\"ignored.rs\"); }}",
            marker
        ),
    )
    .unwrap();
    fs::write(
        f.repository.path().join("src/ignored.rs"),
        "NOT_VALID_RUST_MUST_NOT_READ",
    )
    .unwrap();
    let client = client();
    for cargo in [false, true] {
        for rust in [false, true] {
            let (_process, url) = start(&f, cargo, rust);
            let value = get(&client, &url).await;
            assert_eq!(
                value["cargo_dependencies"]["status"],
                if cargo { "ready" } else { "disabled" }
            );
            let source = &value["rust_call_sites"];
            assert_eq!(
                source["status"],
                if rust { "ready" } else { "disabled" },
                "{source}"
            );
            if rust {
                let calls = source["graph"]["calls"].as_array().unwrap();
                assert_eq!(calls.len(), 3);
                assert_eq!(calls[0]["name"], "std::fs::write");
                assert_eq!(source["graph"]["files"][0]["external_modules"], 1);
                assert_eq!(source["graph"]["files"][0]["unexpanded_macros"], 2);
            } else {
                assert!(source["graph"].is_null());
            }
            let body = value.to_string();
            for private in [
                "PRIVATE_BODY",
                "MACRO_ARGUMENT",
                "hidden_call",
                "NOT_VALID_RUST",
                "BUILD_SCRIPT",
                "must-not-exist",
            ] {
                assert!(!body.contains(private), "不应输出正文: {private}");
            }
            assert!(!marker.exists());
        }
    }
}

#[tokio::test]
async fn real_process_survives_bounded_adversarial_syntax_and_can_retry() {
    let f = fixture::Fixture::new();
    let path = f.repository.path().join("src/source.rs");
    fs::write(&path, "fn initial() { ready(); }").unwrap();
    let (mut process, url) = start(&f, false, true);
    let client = client();
    assert_eq!(
        get(&client, &url).await["rust_call_sites"]["status"],
        "ready"
    );
    let cases = [
        format!("fn f() {{ let _ = {}true; }}", "!".repeat(32_000)),
        format!(
            "fn f() {{ let _ = {}0{}; }}",
            "(".repeat(16_000),
            ")".repeat(16_000)
        ),
        format!("fn f() {{ {} }}", "x();".repeat(8_000)),
        format!("fn f() {{ x{}; }}", ".f()".repeat(8_000)),
        format!("fn f() {{ let _ = {}0; }}", "a+".repeat(16_000)),
        format!("fn f() {{ {}", "{".repeat(32_000)),
    ];
    for content in cases {
        assert!(content.len() <= 64 * 1024);
        fs::write(&path, content).unwrap();
        let value = get(&client, &url).await;
        let source = &value["rust_call_sites"];
        assert!(
            matches!(source["status"].as_str(), Some("invalid" | "unsupported")),
            "{source}"
        );
        assert!(source["graph"].is_null());
        assert!(source["observed_at"].is_null());
        assert!(process.child.try_wait().unwrap().is_none());
        fs::write(&path, "fn recovered() { usable(); }").unwrap();
        let recovered = get(&client, &url).await;
        assert_eq!(recovered["rust_call_sites"]["status"], "ready");
        assert_eq!(
            recovered["rust_call_sites"]["graph"]["calls"][0]["name"],
            "usable"
        );
    }
}
