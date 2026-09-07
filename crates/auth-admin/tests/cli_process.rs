//! 实际产品二进制通过真实PTY验证，不使用人类身份测试工厂或真实秘密。

#[cfg(unix)]
#[test]
fn actual_cli_process_enrolls_once_without_secret_output() {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/cli_process.py");
    let output = std::process::Command::new("python3")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_studiolo-auth-admin"))
        .env_remove("STUDIOLO_TOTP_SEED_HEX")
        .output()
        .expect("实际CLI进程契约需要Python3标准库与PTY，不允许静默跳过");
    assert!(
        output.status.success(),
        "CLI进程契约失败：{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
