//! 秘密驻留面的固定分类、真实内核回读与真实子进程外部检视。
//!
//! 这里刻意不满足于「调用过某个系统调用」：核心转储限额、可转储属性、锁定页数
//! 都从内核回读，环境槽位的结论由本进程当下**确实还读不读得到**来反证，
//! 同 UID 外部读取由另一个真实进程去读活着的子进程来证伪。
//!
//! 完整的 `harden()` 只在**真实子进程**里跑：`mlockall(FUTURE)` 会影响其后所有
//! 映射，不能加到共享的测试进程上；子进程同时承担幂等性与回读证据两件事。
//!
//! 夹具里只有固定的非秘密标记，绝不含任何真实种子、验证码或口令。

use super::*;
use rustix::process::{Resource, getrlimit};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    time::Duration,
};

/// 置位后本测试二进制作为探针运行；产品源码没有这个入口。
const PROBE_VARIABLE: &str = "STUDIOLO_PROCESS_SAFETY_PROBE";
/// 探针用的环境槽位；名字与产品槽位不同，避免误接真实注入路径。
const MARKER_VARIABLE: &str = "STUDIOLO_PROCESS_SAFETY_MARKER";
/// 固定的非秘密标记：只需要一段可在内存与 `/proc` 里检索的字节，不是秘密。
const MARKER_VALUE: &str = "fictional-residency-marker-not-a-secret";

// ---------------------------------------------------------------------------
// 固定分类：措施集合封闭，摘要只含措施名与分类
// ---------------------------------------------------------------------------

#[test]
fn every_measure_owns_one_slot_and_the_summary_lists_all_of_them_in_order() {
    for (slot, measure) in Measure::ORDER.into_iter().enumerate() {
        assert_eq!(measure.index(), slot, "执行顺序与结果下标必须一一对应");
    }
    let outcomes = [
        Outcome::Applied(Applied::CoreDumpLimitZeroed),
        Outcome::Unsupported(Unsupported::EnvironmentBlockReadableBySameUser),
        Outcome::Applied(Applied::CoreDumpLimitZeroed),
        Outcome::Unsupported(Unsupported::EnvironmentBlockShieldedByInspection),
    ];
    let residency = Residency::new(outcomes);
    for measure in Measure::ORDER {
        assert_eq!(residency.outcome(measure), outcomes[measure.index()]);
    }
    let summary = residency.to_string();
    for measure in Measure::ORDER {
        assert!(
            summary.contains(&measure.to_string()),
            "摘要漏掉了 {measure}"
        );
    }
    assert_eq!(summary.matches('；').count(), Measure::COUNT - 1);
    assert!(summary.contains("已生效") && summary.contains("不支持"));
}

#[test]
fn a_refusal_names_the_measure_the_cause_and_states_that_no_secret_was_read() {
    for measure in Measure::ORDER {
        for cause in every_cause() {
            let refusal = Refusal::new(measure, cause);
            let text = refusal.to_string();
            assert!(
                text.contains(&measure.to_string()),
                "拒绝未点名措施：{text}"
            );
            assert!(
                text.ends_with("；未读取秘密"),
                "拒绝未声明未读取秘密：{text}"
            );
            assert_eq!(refusal.cause(), cause);
        }
    }
}

/// 红绿：本平台声明支持却没生效时整体拒绝，绝不降级成「没成功也照跑」。
///
/// 红侧覆盖全部措施与全部失败原因；绿侧证明同一条记录路径在拿到
/// 「已生效」或「本平台不提供」时只分类、不拒绝。
#[test]
fn a_supported_measure_that_did_not_take_effect_refuses_the_whole_hardening() {
    for measure in Measure::ORDER {
        for cause in every_cause() {
            let refusal = record(measure, Err(cause)).expect_err("支持却没生效必须整体拒绝");
            assert_eq!(refusal.cause(), cause);
            assert!(refusal.to_string().contains(&measure.to_string()));
            assert!(refusal.to_string().ends_with("；未读取秘密"));
        }
    }
    assert_eq!(
        record(
            Measure::CoreDumpLimit,
            Ok(Capability::Applied(Applied::CoreDumpLimitZeroed))
        ),
        Ok(Outcome::Applied(Applied::CoreDumpLimitZeroed))
    );
    assert_eq!(
        record(
            Measure::EnvironmentSlot,
            Ok(Capability::Unsupported(
                Unsupported::EnvironmentBlockReadableBySameUser
            ))
        ),
        Ok(Outcome::Unsupported(
            Unsupported::EnvironmentBlockReadableBySameUser
        ))
    );
}

/// 失败原因集合是封闭的，且随平台不同；新增取值必须同时在这里补齐。
#[cfg(target_os = "linux")]
fn every_cause() -> Vec<Cause> {
    vec![Cause::Rejected, Cause::Unconfirmed, Cause::MemoryLockLimit]
}

#[cfg(not(target_os = "linux"))]
fn every_cause() -> Vec<Cause> {
    vec![Cause::Rejected, Cause::Unconfirmed]
}

#[test]
fn the_environment_conclusion_follows_the_measured_inspection_not_the_platform() {
    assert_eq!(
        classify(platform::environment_slot(true)),
        Outcome::Unsupported(Unsupported::EnvironmentBlockShieldedByInspection)
    );
    assert_eq!(
        classify(platform::environment_slot(false)),
        Outcome::Unsupported(Unsupported::EnvironmentBlockReadableBySameUser)
    );
    // 可检视属性没能生效时，环境槽位的结论必须自动退到「同用户仍可读」。
    assert!(shields_external_read(Outcome::Applied(
        Applied::CoreDumpLimitZeroed
    )));
    assert!(!shields_external_read(Outcome::Unsupported(
        Unsupported::EnvironmentBlockReadableBySameUser
    )));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn a_platform_without_the_switches_reports_unsupported_instead_of_imitating_linux() {
    assert_eq!(
        platform::process_inspection(),
        Ok(Capability::Unsupported(Unsupported::NoDumpableSwitch))
    );
    assert_eq!(
        platform::memory_residency(),
        Ok(Capability::Unsupported(Unsupported::NoAddressSpaceLock))
    );
}

#[test]
fn closing_the_core_dump_limit_is_idempotent_and_reads_back_as_zero() {
    // 只影响这个独立测试进程，不修改 cargo 或父 shell；重复收紧必须幂等。
    for _ in 0..2 {
        assert_eq!(
            platform::core_dump_limit(),
            Ok(Capability::Applied(Applied::CoreDumpLimitZeroed))
        );
    }
    let limit = getrlimit(Resource::Core);
    assert_eq!(limit.current, Some(0));
    assert_eq!(limit.maximum, Some(0));
}

// ---------------------------------------------------------------------------
// 真实子进程：内核回读 + 另一个进程的外部检视
// ---------------------------------------------------------------------------

#[test]
fn a_real_child_process_confirms_each_measure_it_claims() {
    let mut child = Command::new(std::env::current_exe().expect("取得本测试二进制路径"))
        .args([
            "--exact",
            "tests::process_safety_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PROBE_VARIABLE, "1")
        .env(MARKER_VARIABLE, MARKER_VALUE)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("秘密驻留面契约需要真实子进程，不允许静默跳过");
    let observations = read_until_ready(&mut child);
    let pid = numeric(&observations, "PID");
    let effective_user = numeric(&observations, "EUID");
    // 外部检视必须在子进程仍然活着时完成，随后立刻收尾，断言留到之后再做。
    let external = external_observations(pid, effective_user);
    let _ = child.kill();
    let _ = child.wait();

    assert_child_readbacks(&observations);
    assert_external_reads(&external, effective_user);
}

fn read_until_ready(child: &mut std::process::Child) -> BTreeMap<String, String> {
    let stdout = child.stdout.take().expect("探针标准输出已接管");
    let mut reader = BufReader::new(stdout);
    let mut observations = BTreeMap::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("读取探针观测结果");
        assert_ne!(read, 0, "探针在给出 READY 之前就结束了：{observations:?}");
        let Some(entry) = line.trim_end().strip_prefix("PROBE ") else {
            continue;
        };
        if entry == "READY" {
            return observations;
        }
        let (key, value) = entry.split_once('=').expect("探针观测结果是 key=value");
        observations.insert(key.to_owned(), value.to_owned());
    }
}

fn observed<'a>(observations: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    observations
        .get(key)
        .unwrap_or_else(|| panic!("探针未给出 {key}：{observations:?}"))
}

fn numeric(observations: &BTreeMap<String, String>, key: &str) -> u32 {
    observed(observations, key)
        .parse()
        .unwrap_or_else(|_| panic!("{key} 必须是数字"))
}

/// 断言子进程自己从内核回读到的事实，而不是它调用过哪些函数。
fn assert_child_readbacks(observations: &BTreeMap<String, String>) {
    assert_eq!(observed(observations, "CORE_CURRENT"), "Some(0)");
    assert_eq!(observed(observations, "CORE_MAXIMUM"), "Some(0)");
    // 环境槽位分类为「未清除」，那么此刻进程自己必须确实还读得到它。
    assert_eq!(observed(observations, "MARKER_READABLE"), "true");
    // 同一进程里连着收紧两次必须得到同一个封闭结论，不存在「第二次才生效」。
    assert_eq!(
        observed(observations, "IDEMPOTENT"),
        "true",
        "重复调用 harden() 的结论不稳定"
    );

    match observations.get("RESIDENCY") {
        Some(summary) => {
            assert!(
                !summary.contains(MARKER_VALUE),
                "摘要泄漏了环境值：{summary}"
            );
            assert_eq!(
                observed(observations, "ENVIRONMENT_CONSISTENT"),
                "true",
                "环境槽位的固定分类与进程实际读到的内容不一致"
            );
            assert_measure_classification(summary);
            assert_locked_pages(observations);
        }
        // 唯一允许的失败是可锁定内存上限不足；那时产品也不会读取秘密。
        None => {
            let refusal = observed(observations, "REFUSAL");
            assert!(
                refusal.contains("RLIMIT_MEMLOCK"),
                "加固失败的原因不在已知封闭集合内：{refusal}"
            );
            assert!(refusal.ends_with("；未读取秘密"));
        }
    }
    assert_inspection_flag(observations);
}

/// 本平台不提供的能力必须以「不支持」出现在被记录的摘要里，不是静默通过。
#[cfg(target_os = "linux")]
fn assert_measure_classification(summary: &str) {
    for measure in Measure::ORDER {
        assert!(
            summary.contains(&measure.to_string()),
            "摘要漏项：{summary}"
        );
    }
    // Linux 只有环境槽位一项不提供；其余三项必须是「已生效」。
    assert_eq!(summary.matches("不支持：").count(), 1, "{summary}");
    assert_eq!(summary.matches("已生效：").count(), 3, "{summary}");
}

#[cfg(not(target_os = "linux"))]
fn assert_measure_classification(summary: &str) {
    for measure in Measure::ORDER {
        assert!(
            summary.contains(&measure.to_string()),
            "摘要漏项：{summary}"
        );
    }
    // macOS 没有可转储开关、没有可用的地址空间锁定，连同环境槽位共三项不提供；
    // 它们必须被写成「不支持」，不得被抹成「已生效」。
    assert_eq!(summary.matches("不支持：").count(), 3, "{summary}");
    assert_eq!(summary.matches("已生效：").count(), 1, "{summary}");
}

#[cfg(target_os = "linux")]
fn assert_inspection_flag(observations: &BTreeMap<String, String>) {
    // PR_GET_DUMPABLE 的真实回读。
    assert_eq!(observed(observations, "DUMPABLE"), "Ok(NotDumpable)");

    // 自读 `/proc/self/environ` 的结果取决于读者能不能绕过文件权限，**不取决于**
    // `ptrace_may_access`（后者对自己恒放行）：
    // `PR_SET_DUMPABLE(0)` 之后该 proc 文件的 inode 改归 root、模式 0400，
    // VFS 权限检查在 ptrace 检查之前，所以**非 root 进程连自己的环境块都打不开**。
    // 2026-09-17 实测：以 root 跑的容器读到标记，GitHub runner 的非 root 用户 EACCES。
    // 两种结果都必须有断言，不得跳过，也不得把两者塌缩成同一个布尔值。
    let self_environ = observed(observations, "SELF_ENVIRON");
    // 分支自证：下面两种读数都算通过，日志不写清本次是哪一种，
    // 就说不出另一种有没有被真正跑到 —— 那等于两支都没验证。
    eprintln!(
        "秘密驻留面·分支自证：dumpable=0 后自读 /proc/self/environ = {self_environ}\
         （「读到标记」＝ 本次身份能绕过文件权限；「PermissionDenied」＝ 不能绕过）"
    );
    match self_environ {
        // 能绕过文件权限（root / CAP_DAC_OVERRIDE）：读得到，说明内核那份环境块副本
        // 确实仍在 —— 与「环境槽位未清除」的固定分类一致。
        "读到标记" => {}
        // 不能绕过：连自读都被挡住。这比 `docs/m2/secret-residency.md` 记载的保护更强，
        // 但它来自文件权限而不是本模块的能力，所以分类仍然是「未清除」。
        "PermissionDenied" => {}
        other => panic!(
            "自读 /proc/self/environ 只应是「读到标记」或 PermissionDenied，实际是 {other}；\
             「读到但没有标记」意味着有人清了环境块，与固定分类「未清除」矛盾"
        ),
    }
}

#[cfg(not(target_os = "linux"))]
fn assert_inspection_flag(observations: &BTreeMap<String, String>) {
    // 本平台没有可转储属性开关，探针不应凭空给出这项读数。
    assert!(!observations.contains_key("DUMPABLE"));
}

#[cfg(target_os = "linux")]
fn assert_locked_pages(observations: &BTreeMap<String, String>) {
    let locked = observed(observations, "VMLCK_KB");
    let kilobytes: u64 = locked
        .parse()
        .unwrap_or_else(|_| panic!("VmLck 必须是内核给出的数字，实得 {locked}"));
    assert!(kilobytes > 0, "声称已锁定却没有任何锁定页");
}

#[cfg(not(target_os = "linux"))]
fn assert_locked_pages(observations: &BTreeMap<String, String>) {
    assert!(!observations.contains_key("VMLCK_KB"));
}

/// 另一个真实进程读活着的子进程；Linux 上这是同 UID 拒绝的唯一硬证据。
#[cfg(target_os = "linux")]
fn external_observations(pid: u32, _effective_user: u32) -> BTreeMap<String, String> {
    use std::os::unix::fs::MetadataExt;

    let environ = match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(bytes) if contains_marker(&bytes) => "读到标记".to_owned(),
        Ok(_) => "读到但没有标记".to_owned(),
        Err(error) => format!("{:?}", error.kind()),
    };
    let owner = std::fs::metadata(format!("/proc/{pid}"))
        .map(|metadata| metadata.uid().to_string())
        .unwrap_or_else(|error| format!("{:?}", error.kind()));
    BTreeMap::from([
        ("environ".to_owned(), environ),
        ("proc_owner".to_owned(), owner),
    ])
}

/// macOS 没有 `/proc`；同用户读他进程环境要走 `KERN_PROCARGS2`，
/// 其可用性随系统版本与签名状态变化，这里不做不可靠的断言，
/// 平台结论由固定分类（同用户仍可读）与 `docs/m2/secret-residency.md` 给出。
#[cfg(not(target_os = "linux"))]
fn external_observations(_pid: u32, _effective_user: u32) -> BTreeMap<String, String> {
    BTreeMap::new()
}

/// 本进程是否持有 `CAP_SYS_PTRACE`（capability 编号 19）。
///
/// 读取 `dumpable=0` 进程的 `/proc/<pid>/environ`，内核走的是 `ptrace_may_access`
/// 的 `PTRACE_MODE_READ`，要求的是这项 capability **而不是** uid 0。
/// 两者平时一起出现，所以容易被误当成同一件事；但容器默认的 capability 集合
/// 通常**丢掉** `CAP_SYS_PTRACE`，此时容器内的 root 同样读不到。
/// 按 uid 判断会在容器里给出错误结论（2026-09-17 Linux 回归实测撞到）。
///
/// 返回**原始掩码**而不是只返回布尔：分支断言必须能在日志里自证走的是哪一支，
/// 只留一个布尔就说不出「凭什么是这一支」，也就无从判断另一支是否根本没被跑到。
#[cfg(target_os = "linux")]
fn ptrace_capability() -> (String, bool) {
    const CAP_SYS_PTRACE: u32 = 19;
    let raw = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("CapEff:"))
                .map(|value| value.trim().to_owned())
        })
        .unwrap_or_else(|| "读不到".to_owned());
    let held = u64::from_str_radix(&raw, 16).is_ok_and(|mask| mask & (1u64 << CAP_SYS_PTRACE) != 0);
    (raw, held)
}

#[cfg(target_os = "linux")]
fn assert_external_reads(external: &BTreeMap<String, String>, effective_user: u32) {
    let environ = observed(external, "environ");
    let owner = observed(external, "proc_owner");
    // 分支自证：两种身份下这条断言都会「通过」，日志不写清走的是哪一支，
    // 就等于两支都没被验证过。原始 `CapEff` 与选中的分支一并进日志。
    let (capabilities, ptrace) = ptrace_capability();
    eprintln!(
        "秘密驻留面·分支自证：euid={effective_user} CapEff={capabilities} \
         CAP_SYS_PTRACE={} → 走「{}」支；外部实读 environ={environ}，/proc/<pid> 属主={owner}",
        if ptrace { "持有" } else { "未持有" },
        if ptrace {
            "必须读到标记"
        } else {
            "必须 PermissionDenied"
        },
    );
    if ptrace {
        // 持有 CAP_SYS_PTRACE 时这项保护按设计就不成立；本模块从不宣称挡得住它。
        // 断言「确实读得到」而不是跳过，避免空跑成假绿。
        assert_eq!(
            environ, "读到标记",
            "持有 CAP_SYS_PTRACE 时的读取行为与文档记载不符"
        );
    } else {
        // 真正的不变量：没有 CAP_SYS_PTRACE 就读不到别的进程的环境块。
        // 内核这里走的是 `ptrace_may_access` 的 PTRACE_MODE_READ_FSCREDS，
        // 所以连持有 CAP_DAC_OVERRIDE 的容器 root 也一样被拒。
        assert_eq!(environ, "PermissionDenied", "仍读得到子进程环境块");

        // `/proc/<pid>` 目录属主**不是**可移植判据，不能拿来断言。
        // 2026-09-17 同一内核、同一 dumpable=0 子进程实测：
        //   以 root 跑  → 属主 uid=0    外部读 environ EACCES
        //   以 1000 跑  → 属主 uid=1000 外部读 environ EACCES
        // 两种身份下保护都成立，但属主只在前者变成 0。原断言把它当必然结果，
        // 会在非 root 环境（GitHub runner 即是）给出错误结论。
        // 这里只记录读数供排障，不据此判定成败。
        assert!(!owner.is_empty(), "探针应给出 /proc/<pid> 属主读数供排障");
    }
}

#[cfg(not(target_os = "linux"))]
fn assert_external_reads(external: &BTreeMap<String, String>, _effective_user: u32) {
    assert!(external.is_empty());
}

#[cfg(target_os = "linux")]
fn contains_marker(bytes: &[u8]) -> bool {
    bytes
        .windows(MARKER_VALUE.len())
        .any(|window| window == MARKER_VALUE.as_bytes())
}

/// 探针：在真实子进程里跑完整加固，把内核回读结果原样交给父进程。
#[test]
fn process_safety_probe() {
    if std::env::var_os(PROBE_VARIABLE).is_none() {
        return;
    }
    let hardened = harden();
    // 幂等性只在这个一次性子进程里验证：`mlockall(FUTURE)` 影响其后所有映射，
    // 不能加到共享的测试进程上。
    let idempotent = harden() == hardened;
    let core = getrlimit(Resource::Core);
    let mut lines = vec![
        format!("EUID={}", rustix::process::geteuid().as_raw()),
        format!("CORE_CURRENT={:?}", core.current),
        format!("CORE_MAXIMUM={:?}", core.maximum),
        format!("IDEMPOTENT={idempotent}"),
        format!(
            "MARKER_READABLE={}",
            std::env::var_os(MARKER_VARIABLE).is_some()
        ),
    ];
    match &hardened {
        Ok(residency) => {
            lines.push(format!("RESIDENCY={residency}"));
            lines.push(format!(
                "ENVIRONMENT_CONSISTENT={}",
                environment_classification_matches_reality(residency)
            ));
        }
        Err(refusal) => lines.push(format!("REFUSAL={refusal}")),
    }
    lines.extend(kernel_observations());
    lines.push(format!("PID={}", std::process::id()));
    lines.push("READY".to_owned());

    let mut stdout = std::io::stdout();
    // 先换行：测试运行器打印用例名时不带换行，否则第一条观测会粘在它后面，
    // 前缀被破坏、父进程直接漏读一项。
    writeln!(stdout).expect("探针必须能写出观测结果");
    for line in lines {
        writeln!(stdout, "PROBE {line}").expect("探针必须能写出观测结果");
    }
    stdout.flush().expect("探针必须刷出观测结果");
    // 父进程要在本进程活着时做外部检视，完成后会直接终止本进程。
    std::thread::sleep(Duration::from_secs(20));
    panic!("父进程未在 20 秒内完成外部检视");
}

/// 反证环境槽位的分类：说「未清除」就必须真读得到，说「已清除」就必须读不到。
fn environment_classification_matches_reality(residency: &Residency) -> bool {
    let readable = std::env::var_os(MARKER_VARIABLE).is_some();
    match residency.outcome(Measure::EnvironmentSlot) {
        Outcome::Unsupported(_) => readable,
        Outcome::Applied(_) => !readable,
    }
}

#[cfg(target_os = "linux")]
fn kernel_observations() -> Vec<String> {
    vec![
        format!("DUMPABLE={:?}", rustix::process::dumpable_behavior()),
        format!(
            "VMLCK_KB={}",
            status_kilobytes("VmLck").map_or_else(|| "未知".to_owned(), |value| value.to_string())
        ),
        // 三态，不能塌缩：「读不到」与「读得到但没有标记」是两个完全不同的事实，
        // 前者说明 dumpable=0 连自读都挡住了，后者说明有人把环境块清了。
        format!(
            "SELF_ENVIRON={}",
            match std::fs::read("/proc/self/environ") {
                Ok(bytes) if contains_marker(&bytes) => "读到标记".to_owned(),
                Ok(_) => "读到但没有标记".to_owned(),
                Err(error) => format!("{:?}", error.kind()),
            }
        ),
    ]
}

#[cfg(not(target_os = "linux"))]
fn kernel_observations() -> Vec<String> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn status_kilobytes(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix(field)?
            .strip_prefix(':')?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}
