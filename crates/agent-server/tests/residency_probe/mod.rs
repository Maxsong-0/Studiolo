//! 副本内存契约共用的探针脚手架（T-185 `replica_memory_contract`、T-215 `replica_capacity_contract`）。
//!
//! 生产 agent 按秘密驻留面契约 `mlockall` 了整个地址空间。这会影响其后所有映射，不能加到共享的测试进程上
//! （同 process-safety 的约定）：测试二进制以探针身份重跑自己，子进程像生产一样加固、起真实 loopback 服务、
//! 量自己的读数，按「键=值」打到标准输出，交给父进程判定。
use std::{
    ffi::OsStr,
    future::Future,
    process::Command,
    time::{Duration, Instant},
};

/// 探针的标准输出；按「键=值」取读数。
pub struct Readings {
    stdout: String,
    stderr: String,
}

impl Readings {
    /// 整数读数（KiB、毫秒）。
    pub fn number(&self, key: &str) -> u64 {
        self.text(key)
            .parse()
            .unwrap_or_else(|_| panic!("探针的 {key} 不是整数：\n{}\n{}", self.stdout, self.stderr))
    }

    /// 原样的读数文字。libtest 会把探针的第一行输出接在「test <探针名> ... 」后面，所以按「键=」切分，不按行首匹配。
    pub fn text(&self, key: &str) -> &str {
        self.stdout
            .lines()
            .find_map(|line| line.split_once(&format!("{key}=")))
            .map(|(_, value)| value.trim())
            .unwrap_or_else(|| panic!("探针没有报告 {key}：\n{}\n{}", self.stdout, self.stderr))
    }
}

/// 以探针身份重跑本测试二进制里名为 `name` 的测试：置位 `flag` 让它真的去量，再带上 `env`。探针失败就判红。
///
/// glibc 的 malloc 调参会改变 arena 行为（比如 `MALLOC_ARENA_MAX=1` 会让回归也量不出来），探针一律不继承。
pub fn run(name: &str, flag: &str, env: &[(&str, &OsStr)]) -> Readings {
    let mut probe = Command::new(std::env::current_exe().expect("取得测试二进制路径"));
    probe
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(flag, "1");
    for (key, value) in env {
        probe.env(key, value);
    }
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|key| key.starts_with("MALLOC_") || key == "GLIBC_TUNABLES")
        {
            probe.env_remove(key);
        }
    }
    let output = probe.output().expect("启动探针子进程");
    let readings = Readings {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    assert!(
        output.status.success(),
        "探针失败：\n{}\n{}",
        readings.stdout,
        readings.stderr
    );
    readings
}

/// 探针一侧：与生产一样锁定地址空间，再在 4 条工作线程的运行时上跑 `body`。
pub fn harden_then_block_on<F: Future>(body: F) -> F::Output {
    let residency = studiolo_process_safety::harden().unwrap_or_else(|refusal| {
        panic!(
            "与生产一样锁定地址空间失败（memlock 上限不足时按设计拒绝，不得据此跳过）：{refusal}"
        )
    });
    eprintln!("秘密驻留面：{residency}");
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("建立运行时")
        .block_on(body)
}

/// 本进程 `/proc/self/status` 里的一项（KiB），如 `RssAnon:`、`RssFile:`、`VmHWM:`。
/// 锁定后 glibc 释放的堆页退不回去，仍算在 `RssAnon` 里，cgroup 记账看的也是它。
pub fn status_kib(name: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .expect("读取 /proc/self/status")
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .unwrap_or_else(|| panic!("/proc/self/status 里有 {name}"))
}

/// 发请求并读完响应体（像真实客户端一样让服务端把这次供给完整走完），返回状态码与时延。
pub async fn timed(
    request: reqwest::RequestBuilder,
) -> reqwest::Result<(reqwest::StatusCode, Duration)> {
    let start = Instant::now();
    let response = request.send().await?;
    let status = response.status();
    response.bytes().await?;
    Ok((status, start.elapsed()))
}
