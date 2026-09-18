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
            "process_safety::tests::process_safety_probe",
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
    // PR_GET_DUMPABLE 的真实回读；同时记录内核环境块副本仍在这一事实。
    assert_eq!(observed(observations, "DUMPABLE"), "Ok(NotDumpable)");
    assert_eq!(
        observed(observations, "SELF_ENVIRON_MARKER"),
        "true",
        "进程自读 /proc/self/environ 的结果必须与「未清除」的分类一致"
    );
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

#[cfg(target_os = "linux")]
fn assert_external_reads(external: &BTreeMap<String, String>, effective_user: u32) {
    let environ = observed(external, "environ");
    let owner = observed(external, "proc_owner");
    if effective_user == 0 {
        // 以 root 跑时这项保护按设计就不成立；本模块从不宣称挡得住 root。
        // 断言「root 确实读得到」而不是跳过，避免在容器里空跑成假绿。
        assert_eq!(environ, "读到标记", "root 读取行为与文档记载不符");
    } else {
        assert_eq!(environ, "PermissionDenied", "同用户仍读得到子进程环境块");
        assert_eq!(owner, "0", "不可转储进程的 /proc 目录应改归 root");
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
        format!(
            "SELF_ENVIRON_MARKER={}",
            std::fs::read("/proc/self/environ").is_ok_and(|bytes| contains_marker(&bytes))
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
