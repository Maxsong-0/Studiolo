//! `studiolo-agent` 的秘密驻留面：由**实际产品二进制**证明，不用替身或内部 Seam。
//!
//! 这里钉住的是 T-065 修掉的真实缺口：`studiolo-agent` 是第四个持有明文秘密的程序
//! （`admin type-issue` 铸出的 32 字节类型 token 是 MCP 供给面的全部身份，bearer 每次
//! 请求都在内存里），却因为不能依赖 `studiolo-host-config`（依赖策略不允许它把认证栈
//! 拉进供给面）而**一项收紧都没有**，且这就是线上 4400 端口正在跑的那个进程。
//!
//! 因此本文件不满足于「调用过某个函数」：
//!
//! - 逐项分类从真实二进制的标准错误上数出来，一次运行恰好记录一次；
//! - **与子命令无关**：`--help` 与一条注定失败的管理命令都必须先收紧再做事；
//! - Linux 上另起一个真实 `serve` 进程，由**父进程从内核回读**它的锁定页数与
//!   `/proc/<pid>/environ` 的可读性——那是本进程自己说了不算的外部证据；
//! - 本平台不提供的能力必须以「不支持」出现在被记录的摘要里，按平台逐项核对条数，
//!   不允许被抹成「已生效」，也不允许静默跳过。
//!
//! 夹具只用独立临时目录与虚构笔记，绝不含真实种子、token 或口令。

#[cfg(target_os = "linux")]
use std::io::{BufRead as _, BufReader};
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// 产品在最早的无条件路径上写出的固定前缀；数它的条数即可核对「收紧一次记录一次」。
const RECORD_PREFIX: &str = "秘密驻留面：";
/// 封闭的四项措施名；摘要漏掉任何一项都说明有措施被静默跳过。
const MEASURES: [&str; 4] = ["核心转储限额", "进程可检视属性", "秘密页驻留", "环境槽位"];

/// 解析一个既能在开发机（编译期烘焙路径可用）也能在发布包里（源码解压到另一台机器上，
/// 烘焙路径不存在）定位到的实际文件：显式环境覆盖 → 编译期路径 → 相对当前源码根。
///
/// 三条都落空时直接 panic 并说明如何指定，绝不把「找不到」当作跳过。
fn agent_binary() -> PathBuf {
    // 仅测试运行器可指定随发布包搬运的生产程序；不传入产品环境。
    if let Some(value) = std::env::var_os("STUDIOLO_TEST_AGENT_BIN") {
        let path = PathBuf::from(value);
        assert!(
            path.is_absolute() && path.is_file(),
            "STUDIOLO_TEST_AGENT_BIN 必须是已存在的绝对文件路径：{}",
            path.display()
        );
        return path;
    }
    let baked = PathBuf::from(env!("CARGO_BIN_EXE_studiolo-agent"));
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

/// 跑一次实际产品二进制，返回 (退出是否成功, 标准输出, 标准错误)。
fn run_agent(arguments: &[&str]) -> (bool, String, String) {
    let output = Command::new(agent_binary())
        .args(arguments)
        .output()
        .expect("秘密驻留面契约需要运行实际产品二进制，不允许缺失时静默跳过");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 从标准错误上取出**唯一**那条逐项分类；多一条说明又有人加了一份私有收紧，
/// 少一条说明分类被藏了起来。
fn only_record(stderr: &str, scenario: &str) -> String {
    assert_eq!(
        stderr.matches(RECORD_PREFIX).count(),
        1,
        "{scenario} 的逐项分类记录条数不是 1：\n{stderr}"
    );
    stderr
        .lines()
        .find_map(|line| {
            line.split_once(RECORD_PREFIX)
                .map(|(_, rest)| rest.to_owned())
        })
        .unwrap_or_else(|| panic!("{scenario} 没有记录逐项分类：\n{stderr}"))
}

/// 四项措施全在，且本平台不提供的那几项如实写成「不支持」。
fn assert_closed_classification(summary: &str, scenario: &str) {
    for measure in MEASURES {
        assert!(summary.contains(measure), "{scenario} 的摘要漏掉 {measure}");
    }
    assert_eq!(
        summary.matches('；').count(),
        MEASURES.len() - 1,
        "{scenario} 的摘要项数与封闭措施集合不符：{summary}"
    );
    assert_platform_counts(summary, scenario);
}

/// Linux 只有环境槽位一项不提供；其余三项必须是「已生效」。
#[cfg(target_os = "linux")]
fn assert_platform_counts(summary: &str, scenario: &str) {
    assert_eq!(
        summary.matches("已生效：").count(),
        3,
        "{scenario}：{summary}"
    );
    assert_eq!(
        summary.matches("不支持：").count(),
        1,
        "{scenario}：{summary}"
    );
}

/// macOS 没有可转储开关、没有可用的地址空间锁定，连同环境槽位共三项不提供；
/// 它们必须被写成「不支持」，不得被抹成「已生效」，也不得整项消失。
#[cfg(not(target_os = "linux"))]
fn assert_platform_counts(summary: &str, scenario: &str) {
    assert_eq!(
        summary.matches("已生效：").count(),
        1,
        "{scenario}：{summary}"
    );
    assert_eq!(
        summary.matches("不支持：").count(),
        3,
        "{scenario}：{summary}"
    );
}

// ---------------------------------------------------------------------------
// 与子命令无关：成功路径与失败路径都先收紧再做事
// ---------------------------------------------------------------------------

#[test]
fn the_real_binary_hardens_and_records_once_before_it_even_prints_its_usage() {
    let (success, stdout, stderr) = run_agent(&["--help"]);
    assert!(success, "--help 应当成功：\n{stdout}\n{stderr}");
    let summary = only_record(&stderr, "--help");
    assert_closed_classification(&summary, "--help");
    // 记录只走标准错误：标准输出留给管理子命令的非秘密结果，允许重定向与管道。
    assert!(
        stdout.contains("studiolo-agent") && !stdout.contains(RECORD_PREFIX),
        "逐项分类不得混进标准输出：\n{stdout}"
    );
}

/// 收紧挂在**入口**而不是挂在某条命令上：一条注定失败的管理命令同样先收紧、先记录。
///
/// 这一条正是「修一个漏一片」的复发探针——把收紧埋进某个子命令的装配里，
/// 别的路径就会一项也没有，而它们同样在内存里持有 bearer 与 token。
#[test]
fn a_command_that_fails_immediately_still_hardened_and_recorded_first() {
    let missing = "/nonexistent-studiolo-t065-data";
    assert!(!Path::new(missing).exists(), "夹具路径必须不存在");
    let (success, stdout, stderr) = run_agent(&["admin", "types", "--data", missing]);
    assert!(!success, "私有库不存在时必须失败：\n{stdout}\n{stderr}");
    let summary = only_record(&stderr, "失败的 admin types");
    assert_closed_classification(&summary, "失败的 admin types");
    let recorded = stderr
        .find(RECORD_PREFIX)
        .expect("失败路径也必须记录逐项分类");
    let failed = stderr
        .find("agent 私有库")
        .or_else(|| stderr.find("私有"))
        .expect("失败路径应当给出拒绝原因");
    assert!(
        recorded < failed,
        "命令失败的诊断排在了加固记录之前：\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// 外部证据：父进程回读一个真实 serve 进程的内核状态（Linux）
// ---------------------------------------------------------------------------

/// Linux 上由**另一个进程**去读活着的真实 `studiolo-agent serve`：
/// 锁定页数与环境块可读性都不是该进程自己的说法，而是内核的回答。
#[cfg(target_os = "linux")]
#[test]
fn another_process_confirms_the_running_agent_really_locked_its_pages_and_closed_inspection() {
    let fixture = ServeFixture::start();
    let pid = fixture.pid();

    // 锁定页数：`/proc/<pid>/status` 世界可读，dumpable=0 不影响它。
    let locked = status_kilobytes(pid, "VmLck")
        .unwrap_or_else(|| panic!("读不到 /proc/{pid}/status 的 VmLck"));
    assert!(locked > 0, "真实 serve 进程声称已锁定却没有任何锁定页");

    // 可检视属性：读 dumpable=0 进程的环境块走 `ptrace_may_access` 的
    // PTRACE_MODE_READ_FSCREDS，要求 CAP_SYS_PTRACE 而**不是** uid 0 ——
    // 容器默认丢掉该 capability，按 uid 判断会在容器里给出错误结论。
    let environ = match fs::read(format!("/proc/{pid}/environ")) {
        Ok(_) => "读得到".to_owned(),
        Err(error) => format!("{:?}", error.kind()),
    };
    // 分支自证：两种身份下这条用例都会「通过」，日志不写清走的是哪一支，
    // 就等于两支都没被验证过。把内核给出的原始 `CapEff` 与选中的分支一并打出来，
    // 事后只看日志即可判定本次跑到的究竟是哪一面。
    let (capabilities, ptrace) = ptrace_capability();
    eprintln!(
        "秘密驻留面·分支自证：euid={} CapEff={capabilities} CAP_SYS_PTRACE={} \
         → 走「{}」支；实读 /proc/{pid}/environ = {environ}",
        effective_user(),
        if ptrace { "持有" } else { "未持有" },
        if ptrace {
            "必须读得到"
        } else {
            "必须 PermissionDenied"
        },
    );
    if ptrace {
        // 持有该 capability 时这项保护按设计就不成立；本模块从不宣称挡得住它。
        // 断言「确实读得到」而不是跳过，避免空跑成假绿。
        assert_eq!(
            environ, "读得到",
            "持有 CAP_SYS_PTRACE 时的读取行为与契约记载不符"
        );
    } else {
        assert_eq!(
            environ, "PermissionDenied",
            "同用户的另一个进程仍读得到 studiolo-agent 的环境块"
        );
    }

    // 核心转储限额：`/proc/<pid>/limits` 同样由内核给出，软硬两档都必须是 0。
    let limits = fs::read_to_string(format!("/proc/{pid}/limits"))
        .unwrap_or_else(|error| panic!("读不到 /proc/{pid}/limits：{error}"));
    let core = limits
        .lines()
        .find(|line| line.starts_with("Max core file size"))
        .unwrap_or_else(|| panic!("/proc/{pid}/limits 没有核心转储一行：\n{limits}"));
    let fields = core
        .trim_start_matches("Max core file size")
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        ["0", "0"],
        "真实 serve 进程的核心转储限额不是 0：{core}"
    );
}

/// 本进程的 `CapEff` 原始读数，以及其中 `CAP_SYS_PTRACE`（capability 编号 19）位。
///
/// 返回原始掩码而不是只返回布尔：分支断言必须能在日志里自证走的是哪一支，
/// 只留一个布尔就说不出「凭什么是这一支」，也就无从判断另一支是否根本没被跑到。
/// 读不到 `/proc/self/status` 时如实记为「读不到」，并按未持有处理。
#[cfg(target_os = "linux")]
fn ptrace_capability() -> (String, bool) {
    const CAP_SYS_PTRACE: u32 = 19;
    let raw = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("CapEff:"))
                .map(|value| value.trim().to_owned())
        })
        .unwrap_or_else(|| "读不到".to_owned());
    let held =
        u64::from_str_radix(&raw, 16).is_ok_and(|mask| mask & (1_u64 << CAP_SYS_PTRACE) != 0);
    (raw, held)
}

/// 有效 uid，取自 `/proc/self/status` 的 `Uid:` 第二列（本 crate 的测试不依赖 rustix）。
///
/// 它**只进自证日志、不参与判定**：判定依据始终是 capability 而不是 uid，
/// 容器里的 root 通常并不持有 `CAP_SYS_PTRACE`。两者一起记录才看得出这一点。
#[cfg(target_os = "linux")]
fn effective_user() -> String {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:"))
                .and_then(|value| value.split_whitespace().nth(1).map(str::to_owned))
        })
        .unwrap_or_else(|| "读不到".to_owned())
}

#[cfg(target_os = "linux")]
fn status_kilobytes(pid: u32, field: &str) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix(field)?
            .strip_prefix(':')?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

/// 一个真实运行的 `studiolo-agent serve`：独立临时目录、虚构笔记、回环端口。
///
/// 释放时终止子进程；临时目录随 `TempDir` 一起清理，不留残留。
#[cfg(target_os = "linux")]
struct ServeFixture {
    _temp: tempfile::TempDir,
    child: std::process::Child,
    // 端口占位在 Drop 收走子进程之后才释放（T-171）。
    _reservation: tokio::net::TcpSocket,
}

#[cfg(target_os = "linux")]
impl ServeFixture {
    fn start() -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::TempDir::new_in(fs::canonicalize("/tmp").expect("/tmp 可解析"))
            .expect("建立独立临时目录");
        let vault = temp.path().join("vault");
        let note = vault.join("项目/Alpha/00-总览/INDEX.md");
        fs::create_dir_all(note.parent().expect("笔记有父目录")).expect("建立虚构白名单目录");
        fs::write(&note, "# Alpha · 总览\n\n虚构内容，不是秘密。\n").expect("写入虚构笔记");
        let data = temp.path().join("agent-data");
        for directory in [&data, &data.join("audit")] {
            fs::create_dir_all(directory).expect("建立私有目录");
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("收紧私有目录权限");
        }

        // 初始化也走实际产品二进制：这条路径同样必须先收紧、先记录。
        let (success, stdout, stderr) = run_agent(&[
            "admin",
            "init",
            "--data",
            data.to_str().expect("临时目录路径是规范 UTF8"),
        ]);
        assert!(success, "admin init 失败：\n{stdout}\n{stderr}");
        only_record(&stderr, "admin init");

        // 端口占位与子进程同寿（T-171）：带 SO_REUSEADDR 只绑定不监听，产品经 tokio 绑定同一端口；
        // 占位期间同机别的进程的自动分配选不到它（Linux 默认 ip_autobind_reuse=0），不会在交接窗口被抢走。
        // 旧写法取号后立即释放，窗口内端口可能落到别的进程手里。产品只接受 1024..=65535，临时端口段满足。
        let reservation = tokio::net::TcpSocket::new_v4().expect("建立端口占位");
        reservation
            .set_reuseaddr(true)
            .expect("占位须允许产品共享绑定");
        reservation
            .bind(([127, 0, 0, 1], 0).into())
            .expect("取一个空闲回环端口");
        let port = reservation.local_addr().expect("占位有本地地址").port();
        let mut child = Command::new(agent_binary())
            .args([
                "serve",
                "--vault",
                vault.to_str().expect("临时目录路径是规范 UTF8"),
                "--allow",
                "项目/Alpha",
                "--data",
                data.to_str().expect("临时目录路径是规范 UTF8"),
                "--port",
                &port.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("启动真实 serve 进程");

        // 等产品自己宣布已在监听；在此之前读 /proc 可能撞上尚未完成的装配。
        let stdout = child.stdout.take().expect("serve 标准输出已接管");
        let mut reader = BufReader::new(stdout);
        let ready = format!("http://127.0.0.1:{port}/mcp");
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("读取 serve 启动输出");
            if read == 0 {
                // 起不来时把子进程的诊断原样带出来，否则只剩一句「结束了」无从排查；
                // 其中也包括那条逐项分类——`RLIMIT_MEMLOCK` 不足正是最可能的原因。
                let mut diagnostics = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    use std::io::Read as _;
                    let _ = stderr.read_to_string(&mut diagnostics);
                }
                let _ = child.kill();
                let status = child.wait().ok();
                panic!("serve 在宣布监听之前就结束了（退出 {status:?}）：\n{diagnostics}");
            }
            if line.contains(&ready) {
                break;
            }
        }
        Self {
            _temp: temp,
            child,
            _reservation: reservation,
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

#[cfg(target_os = "linux")]
impl Drop for ServeFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// 源码守卫：把「最先收紧」「全 crate 只有一份实现」从注释变成可执行约束
// ---------------------------------------------------------------------------

#[test]
fn hardening_is_the_first_thing_the_entry_point_does() {
    let code = without_comments(&read_source("main.rs"));
    let hardened = code
        .find("studiolo_process_safety::harden()")
        .expect("main 必须调用共享的 studiolo_process_safety::harden()");
    let recorded = code
        .find(RECORD_PREFIX)
        .expect("harden() 的逐项分类必须被记录，不能只返回不记录");
    assert!(hardened < recorded);
    // 参数读取与命令分发都必须排在收紧与记录之后；`run(&args)` 之后才有子命令，
    // 所以收紧无法挂在某一条命令上。
    for later in ["std::env::args_os", "studiolo_agent_server::run"] {
        let at = code
            .find(later)
            .unwrap_or_else(|| panic!("main.rs 应当包含 {later}"));
        assert!(recorded < at, "{later} 出现在加固与记录之前");
    }
}

/// 全 crate 只允许存在一份实现：任何自持的部分收紧都是「修一个漏一片」的复发。
#[test]
fn no_source_file_keeps_a_private_copy_of_the_hardening() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0_usize;
    for path in rust_sources(&root) {
        let code = without_comments(&fs::read_to_string(&path).expect("读取本 crate 源码"));
        scanned += 1;
        for syscall in [
            "setrlimit",
            "getrlimit",
            "Resource::Core",
            "set_dumpable_behavior",
            "mlockall",
            "remove_var",
        ] {
            assert!(
                !code.contains(syscall),
                "{} 自行调用了 {syscall}：收紧必须只经 studiolo_process_safety::harden()",
                path.display()
            );
        }
    }
    assert!(scanned >= 20, "源码目录读取不完整，守卫会漏扫");
}

fn read_source(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name))
        .expect("读取指定本 crate 源码")
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("读取源码目录") {
            let path = entry.expect("读取源码目录条目").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|value| value == "rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

/// 去掉整行注释：文档里会举例提到这些调用名，不能被守卫误判为调用。
fn without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
