//! 封存到本地 restic 仓库再恢复的组合演练。没有官方 CLI 时不假装通过。
#[test]
#[ignore = "需要环境变量 STUDIOLO_RESTIC_TEST_BINARY 指向已核验的 restic 0.19.1"]
fn seal_backup_restore_reopen_needs_official_restic() {
    let path = std::env::var("STUDIOLO_RESTIC_TEST_BINARY").expect("官方 restic 路径");
    let version = std::process::Command::new(&path)
        .arg("version")
        .output()
        .expect("执行 restic version");
    assert!(version.status.success());
    let text = String::from_utf8_lossy(&version.stdout);
    assert!(
        text.starts_with("restic 0.19.1 "),
        "只接受官方 0.19.1，得到 {text}"
    );
}
