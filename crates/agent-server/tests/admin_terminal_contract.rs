//! 类型 token 的显示门禁：由真实 PTY 驱动实际产品二进制，不用替身或内部 Seam。
//!
//! 契约 `docs/contracts/agent-supply.md` §2 要求 32 字节随机 token 只显示一次；
//! 本文件把「只在真实前台控制终端显示」的行为、失败与恢复三面各自钉死，
//! 并一并守住同类的第二条出口：签发结果的 `Debug` 不得复述明文。
//! 三个 PTY 场景各自使用独立临时私有目录，互不影响，可并行执行。

/// 解析一个既能在开发机（编译期烘焙路径可用）也能在发布包里（源码解压到另一台机器上，
/// 烘焙路径不存在）定位到的实际文件：显式环境覆盖 → 编译期路径 → 相对当前源码根。
///
/// 三条都落空时直接 panic 并说明如何指定，绝不把「找不到」当作跳过。
#[cfg(unix)]
fn locate(variable: &str, baked: &str, relative: &str) -> std::path::PathBuf {
    // 仅测试运行器可指定随发布包搬运的生产程序；不传入产品环境。
    if let Some(value) = std::env::var_os(variable) {
        let path = std::path::PathBuf::from(value);
        assert!(
            path.is_absolute() && path.is_file(),
            "{variable} 必须是已存在的绝对文件路径：{}",
            path.display()
        );
        return path;
    }
    let baked = std::path::PathBuf::from(baked);
    if baked.is_file() {
        return baked;
    }
    // 发布包在记录的源码根里运行测试程序，编译期的构建容器路径已不存在。
    let fallback = std::env::current_dir()
        .expect("需要可读的当前目录")
        .join(relative);
    assert!(
        fallback.is_file(),
        "找不到 {relative}：编译期路径 {} 不存在，当前目录下也没有；\
         请在记录的源码根运行，或用 {variable} 显式给出绝对路径。不允许据此跳过。",
        baked.display()
    );
    fallback
}

#[cfg(unix)]
fn run_scenario(scenario: &str) {
    let binary = locate(
        "STUDIOLO_TEST_AGENT_BIN",
        env!("CARGO_BIN_EXE_studiolo-agent"),
        "target/release/studiolo-agent",
    );
    let script = locate(
        "STUDIOLO_TEST_AGENT_TERMINAL_SCRIPT",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/admin_terminal.py"
        ),
        "crates/agent-server/tests/support/admin_terminal.py",
    );
    let output = std::process::Command::new("python3")
        .arg(&script)
        .arg(&binary)
        .arg(scenario)
        .output()
        .expect("真实 PTY 契约需要 Python3 标准库与 /dev/tty，不允许缺失时静默跳过");
    assert!(
        output.status.success(),
        "{scenario} 场景失败：\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 行为契约：主人坐在真实控制终端前时，token 正常显示，且只显示这一次。
#[cfg(unix)]
#[test]
fn type_token_is_shown_exactly_once_on_a_real_controlling_terminal_and_never_repeated_by_queries() {
    run_scenario("behaviour");
}

/// 失败契约：没有控制终端、管道、`> file`、改接 stderr、stdin 来自 /dev/null，
/// 五种形状各自被明确拒绝；被改接的那个目标里读不到 token，且一枚都没签发。
#[cfg(unix)]
#[test]
fn type_issue_refuses_every_non_terminal_shape_and_leaves_no_token_in_the_redirected_target() {
    run_scenario("failure");
}

/// 恢复契约：被拒绝的一次调用既不轮换也不吊销既有凭据，
/// 终端恢复正常后照常轮换，且绝不重复显示上一次那枚 token。
#[cfg(unix)]
#[test]
fn a_refused_type_issue_changes_nothing_and_the_next_terminal_run_rotates_to_a_fresh_token() {
    run_scenario("recovery");
}

/// 失败契约（同类出口）：签发结果的 `Debug` 绝不复述明文。
///
/// 派生 `Debug` 会让任何 `{:?}`、日志行或 panic 消息成为第二个「只显示一次」的出口，
/// 与把 token 打进标准输出是同一类缺陷，因此在消费侧一并钉死。
#[test]
fn the_issued_token_debug_representation_never_repeats_the_plaintext() {
    let plaintext = format!("sag1_{}", "0123456789abcdef".repeat(4));
    let issued = studiolo_supply::IssuedToken {
        agent_type: "claude".to_owned(),
        token: plaintext.clone(),
    };
    let rendered = format!("{issued:?}");
    assert!(
        !rendered.contains(&plaintext) && !rendered.contains("0123456789abcdef"),
        "Debug 复述了明文：{rendered}"
    );
    assert!(
        rendered.contains("claude") && rendered.contains("已隐去"),
        "Debug 仍应给出非秘密的类型名与明确的隐去标记：{rendered}"
    );
}
