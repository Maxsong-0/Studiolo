/// 定位实际产品二进制，开发机（编译期路径可用）和发布包（源码解压到另一台机器上、编译期路径不存在）都能找到：
/// 显式环境覆盖 → 编译期路径 → 相对当前源码根。三条都落空时 panic 并说明如何指定，绝不把「找不到」当作跳过。
fn agent_binary() -> std::path::PathBuf {
    // 仅测试运行器可指定随发布包搬运的生产程序（`scripts/run-agent-tests.sh`）；不传入产品环境。
    if let Some(value) = std::env::var_os("STUDIOLO_TEST_AGENT_BIN") {
        let path = std::path::PathBuf::from(value);
        assert!(
            path.is_absolute() && path.is_file(),
            "STUDIOLO_TEST_AGENT_BIN 必须是已存在的绝对文件路径：{}",
            path.display()
        );
        return path;
    }
    let baked = std::path::PathBuf::from(env!("CARGO_BIN_EXE_studiolo-agent"));
    if baked.is_file() {
        return baked;
    }
    let fallback = std::env::current_dir()
        .expect("需要可读的当前目录")
        .join("target/release/studiolo-agent");
    assert!(
        fallback.is_file(),
        "找不到 studiolo-agent：编译期路径 {} 不存在，当前目录下也没有；\
         请在记录的源码根运行，或用 STUDIOLO_TEST_AGENT_BIN 显式给出绝对路径。不允许据此跳过。",
        baked.display()
    );
    fallback
}
