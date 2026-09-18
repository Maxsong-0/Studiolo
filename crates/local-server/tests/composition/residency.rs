//! 秘密驻留面在组合根**无条件**收紧的可执行证据。
//!
//! 这里钉住的是 T-050 复审留下的真实缺口：收紧原先埋在 `factor_input::Inputs::load`
//! 里，只有 `--factor-socket` 模式才走得到，非因子模式一项也没有，而该进程内存里
//! 仍持有 `SessionToken` / `CsrfToken` 明文。所以两条路径都必须各自证明。
//!
//! 证据取自**真实子进程**里的内核回读（`getrlimit`，Linux 另加 `PR_GET_DUMPABLE`
//! 与 `/proc/self/status` 的 `VmLck`），不是「调用过某个函数」；同时数子进程标准
//! 错误上的记录条数，一次装配必须恰好记录一次，杜绝「加固两次只记录一次」。
//!
//! 夹具只用虚构材料与临时目录，绝不含真实种子、验证码或口令。

use super::super::prepare_composed;
use super::{Fixture, test_support};
use crate::options::FactorOptions;
use rustix::process::{Resource, getrlimit};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

/// 置位后本测试二进制作为探针运行；产品源码没有这个入口。
const PROBE_VARIABLE: &str = "STUDIOLO_LOCAL_SERVER_RESIDENCY_PROBE";
/// 产品在组合根写出的固定前缀；数它的条数即可核对「收紧一次记录一次」。
const RECORD_PREFIX: &str = "秘密驻留面：";
/// 封闭的四项措施名；摘要漏掉任何一项都说明有措施被静默跳过。
const MEASURES: [&str; 4] = ["核心转储限额", "进程可检视属性", "秘密页驻留", "环境槽位"];

// ---------------------------------------------------------------------------
// 真实子进程：两种模式各跑一次完整装配，回读内核
// ---------------------------------------------------------------------------

#[test]
fn a_real_child_hardens_and_records_once_on_both_the_factor_and_non_factor_path() {
    for probe in ["non_factor_probe", "factor_probe"] {
        let output = Command::new(std::env::current_exe().expect("取得本测试二进制路径"))
            .args([
                "--exact",
                &format!("startup::tests::residency::{probe}"),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PROBE_VARIABLE, "1")
            .output()
            .expect("秘密驻留面契约需要真实子进程，不允许静默跳过");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "{probe} 未通过：\n{stdout}\n{stderr}"
        );

        let observations = observations(&stdout, probe);
        assert_eq!(
            observed(&observations, "PREPARED", probe),
            "true",
            "{probe} 的装配本身失败了，回读结论不能算数"
        );
        assert_kernel_readbacks(&observations, probe);
        assert_recorded_once(&stderr, probe);
    }
}

fn observations(stdout: &str, probe: &str) -> BTreeMap<String, String> {
    let mut observations = BTreeMap::new();
    for line in stdout.lines() {
        let Some(entry) = line.trim_end().strip_prefix("PROBE ") else {
            continue;
        };
        let (key, value) = entry
            .split_once('=')
            .unwrap_or_else(|| panic!("{probe} 的观测结果不是 key=value：{entry}"));
        observations.insert(key.to_owned(), value.to_owned());
    }
    assert!(!observations.is_empty(), "{probe} 没有给出任何观测结果");
    observations
}

fn observed<'a>(observations: &'a BTreeMap<String, String>, key: &str, probe: &str) -> &'a str {
    observations
        .get(key)
        .unwrap_or_else(|| panic!("{probe} 未给出 {key}：{observations:?}"))
}

/// 断言子进程自己从内核回读到的事实，而不是它调用过哪些函数。
#[cfg(target_os = "linux")]
fn assert_kernel_readbacks(observations: &BTreeMap<String, String>, probe: &str) {
    assert_eq!(observed(observations, "CORE_CURRENT", probe), "Some(0)");
    assert_eq!(observed(observations, "CORE_MAXIMUM", probe), "Some(0)");
    assert_eq!(
        observed(observations, "DUMPABLE", probe),
        "Ok(NotDumpable)",
        "{probe} 的可检视属性没有真正关闭"
    );
    let locked = observed(observations, "VMLCK_KB", probe);
    let kilobytes: u64 = locked
        .parse()
        .unwrap_or_else(|_| panic!("{probe} 的 VmLck 必须是内核给出的数字，实得 {locked}"));
    assert!(kilobytes > 0, "{probe} 声称已锁定却没有任何锁定页");
}

/// macOS 只有核心转储限额这一项真正生效；另外三项必须如实记为「不支持」，
/// 那一面由 [`assert_recorded_once`] 从被记录的摘要上核对。
#[cfg(not(target_os = "linux"))]
fn assert_kernel_readbacks(observations: &BTreeMap<String, String>, probe: &str) {
    assert_eq!(observed(observations, "CORE_CURRENT", probe), "Some(0)");
    assert_eq!(observed(observations, "CORE_MAXIMUM", probe), "Some(0)");
    assert!(
        !observations.contains_key("DUMPABLE") && !observations.contains_key("VMLCK_KB"),
        "{probe} 在没有这两项能力的平台上凭空给出了读数"
    );
}

/// 一次装配收紧一次、记录一次：多一条说明有人又加了一份私有收紧，
/// 少一条说明逐项分类被藏了起来。
fn assert_recorded_once(stderr: &str, probe: &str) {
    assert_eq!(
        stderr.matches(RECORD_PREFIX).count(),
        1,
        "{probe} 的逐项分类记录条数不是 1：\n{stderr}"
    );
    let summary = stderr
        .lines()
        .find_map(|line| line.split_once(RECORD_PREFIX).map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("{probe} 没有记录逐项分类"));
    for measure in MEASURES {
        assert!(summary.contains(measure), "{probe} 的摘要漏掉 {measure}");
    }
    assert!(
        summary.contains("已生效："),
        "{probe} 的摘要没有任何已生效项"
    );
    // 本平台不提供的能力必须以「不支持」出现，不是静默通过；两个平台都至少有
    // 环境槽位这一项。
    assert!(
        summary.contains("不支持："),
        "{probe} 的摘要把不提供的能力抹掉了"
    );
    for forbidden in [
        "3132333435363738393031323334353637383930",
        "3039383736353433323130393837363534333231",
    ] {
        assert!(!summary.contains(forbidden), "{probe} 的摘要泄漏了槽位内容");
    }
}

// ---------------------------------------------------------------------------
// 探针：两种模式各跑一次真实装配，把内核回读交回父进程
// ---------------------------------------------------------------------------

/// 非因子模式：既无会话配置也无因子槽，取钥与秘密槽回调都不许被调用。
#[tokio::test]
async fn non_factor_probe() {
    if std::env::var_os(PROBE_VARIABLE).is_none() {
        return;
    }
    let fixture = Fixture::new(false);
    let prepared = prepare_composed(
        local_only_options(&fixture),
        |_| async { panic!("本机只读模式不得取钥") },
        || panic!("非因子模式不得读取秘密槽"),
    )
    .await;
    report(prepared.is_ok());
}

/// 因子模式：走完整的私有 Unix 入口与固定双槽装配，只用虚构材料。
#[tokio::test]
async fn factor_probe() {
    if std::env::var_os(PROBE_VARIABLE).is_none() {
        return;
    }
    let fixture = Fixture::new(true);
    let prepared = prepare_composed(
        factor_options(&fixture),
        |_| async { Ok(test_support::verifier().await) },
        synthetic_environment,
    )
    .await;
    report(prepared.is_ok());
}

fn local_only_options(fixture: &Fixture) -> crate::options::Options {
    crate::options::parse(
        [
            "--vault".to_owned(),
            path_argument(&fixture.root.join("vault")),
            "--assets".to_owned(),
            path_argument(&fixture.root.join("web")),
            "--port".to_owned(),
            fixture.port.to_string(),
        ]
        .into_iter(),
    )
    .expect("本机只读参数必须可解析")
    .expect("本机只读参数不是 --help")
}

fn factor_options(fixture: &Fixture) -> crate::options::Options {
    let proxy = fixture.root.join("proxy");
    fs::create_dir(&proxy).expect("建立虚构私有代理目录");
    fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).expect("收紧代理目录权限");
    let mut options = fixture.options();
    options.factor = Some(FactorOptions {
        socket: proxy.join("http.sock"),
        replacement_seed_ref: Some("next-seed".into()),
    });
    options
}

/// 与 `composition/factors.rs` 相同的虚构双槽；不是真实种子。
fn synthetic_environment() -> (Option<OsString>, Option<OsString>) {
    (
        Some("3132333435363738393031323334353637383930".into()),
        Some("3039383736353433323130393837363534333231".into()),
    )
}

fn path_argument(path: &Path) -> String {
    path.to_str().expect("临时目录路径是规范 UTF8").to_owned()
}

fn report(prepared: bool) {
    let core = getrlimit(Resource::Core);
    let mut lines = vec![
        format!("PREPARED={prepared}"),
        format!("CORE_CURRENT={:?}", core.current),
        format!("CORE_MAXIMUM={:?}", core.maximum),
    ];
    lines.extend(kernel_observations());

    let mut stdout = std::io::stdout();
    // 先换行：测试运行器打印用例名时不带换行，否则第一条观测会粘在它后面，
    // 前缀被破坏、父进程直接漏读一项。
    writeln!(stdout).expect("探针必须能写出观测结果");
    for line in lines {
        writeln!(stdout, "PROBE {line}").expect("探针必须能写出观测结果");
    }
    stdout.flush().expect("探针必须刷出观测结果");
}

#[cfg(target_os = "linux")]
fn kernel_observations() -> Vec<String> {
    vec![
        format!("DUMPABLE={:?}", rustix::process::dumpable_behavior()),
        format!(
            "VMLCK_KB={}",
            status_kilobytes("VmLck").map_or_else(|| "未知".to_owned(), |value| value.to_string())
        ),
    ]
}

#[cfg(not(target_os = "linux"))]
fn kernel_observations() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn status_kilobytes(field: &str) -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix(field)?
            .strip_prefix(':')?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

// ---------------------------------------------------------------------------
// 源码守卫：把「最先收紧」「全进程只有一份实现」从注释变成可执行约束
// ---------------------------------------------------------------------------

#[test]
fn hardening_is_the_first_thing_each_entry_point_does() {
    let startup = without_comments(&read_source("startup.rs"));
    let hardened = startup
        .find("process_safety::harden()")
        .expect("prepare_composed 必须调用共享的 process_safety::harden()");
    let recorded = startup
        .find("秘密驻留面：")
        .expect("harden() 的逐项分类必须被记录，不能只返回不记录");
    assert!(hardened < recorded);
    // 参数校验、仓库打开、配置加载、因子输入都必须排在收紧与记录之后；
    // `options.factor` 出现在其后，正说明收紧不挂在因子这个条件上。
    for later in [
        "options.port == 0",
        "repository_input::open",
        "Config::load",
        "options.factor",
        "factor_input::Inputs::load",
    ] {
        let at = startup
            .find(later)
            .unwrap_or_else(|| panic!("startup.rs 应当包含 {later}"));
        assert!(recorded < at, "{later} 出现在加固与记录之前");
    }

    let maintenance = without_comments(&read_source("maintenance.rs"));
    let hardened = maintenance
        .find("process_safety::harden()")
        .expect("maintenance::run 必须调用共享的 process_safety::harden()");
    let recorded = maintenance
        .find("秘密驻留面：")
        .expect("维护入口的逐项分类必须被记录");
    assert!(hardened < recorded);
    let parsed = maintenance
        .find("options::parse")
        .expect("maintenance.rs 应当包含 options::parse");
    assert!(recorded < parsed, "维护入口在收紧之前就解析了参数");
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
                "{} 自行调用了 {syscall}：收紧必须只经 \
                 studiolo_process_safety::harden()",
                path.display()
            );
        }
    }
    assert!(scanned >= 40, "源码目录读取不完整，守卫会漏扫");
    for entry in ["startup.rs", "maintenance.rs"] {
        assert!(
            without_comments(&read_source(entry)).contains("studiolo_process_safety::harden()"),
            "{entry} 必须调用共享实现，而不是本地同名函数"
        );
    }
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
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
