//! canonical 整库派生的常驻内存上界（T-186，vault BUG-035）。
//!
//! 生产人侧按秘密驻留面契约 `mlockall` 了整个地址空间，glibc 释放内存时用的
//! `madvise(MADV_DONTNEED)` 对锁定页一律返回 EINVAL：释放掉的页留在分配它的 malloc arena 里；
//! glibc 又给并发分配的线程各开一个 arena。协调器作业（每 30 秒例行检查、监视事件、HTTP 刷新、
//! Bridge 写入之后都会触发）每次完整捕获并重建工作台模型，临时分配随正文总量增长。作业交给 tokio
//! 阻塞线程池时，池里没有空闲线程就新开一条（空闲 10 秒就退出）；生产上黑板维护、Bridge 轮次与
//! HTTP 查询同时占着池里的线程，作业于是轮流落在不同线程、不同 arena 上，每个 arena 都停在一次
//! 派生的峰值：2026-10-16 生产 169 上这样的 arena 堆合计约 300 MiB。
//!
//! `mlockall` 影响其后所有映射，不能加到共享的测试进程上（同 process-safety 的约定）：本测试以
//! 探针身份重跑测试二进制，子进程先像生产一样加固，再起真实协调器；每轮先用别的阻塞任务占满池里
//! 的空闲线程再请求刷新，量自己的常驻匿名内存交给父进程判定。夹具只用虚构笔记，不含真实内容或口令。
//!
//! 量得出回归，靠的是 glibc 给新线程分配 arena 的规则：先取空闲链（已退出线程留下的 arena），
//! 没有就新开一个（上限 8 × 核数），与有没有争用无关。探针里没有线程退出，派生若还交给阻塞池，
//! 每轮都落在新开的线程、新开的 arena 上。换成别的分配器，或 arena 上限低于探针的线程数
//! （约 30 条）时，本测试只会漏报、不会误报。

use std::{process::Command, sync::Arc};

use super::fixture::Fixture;

/// 置位后本测试二进制作为探针运行；产品源码没有这个入口。
const PROBE: &str = "STUDIOLO_CANONICAL_MEMORY_PROBE";
/// 虚构正文约 2.3 MiB：每次派生的临时分配远大于噪声，debug 构建下一次派生也只要一两秒。
const NOTES: usize = 192;
const NOTE_BYTES: usize = 12 * 1024;
/// 换线程刷新的轮数。
const ROUNDS: usize = 6;
/// 第一轮占住的阻塞线程数；之后每轮多一条，盖住此前开过的全部阻塞线程（含上一轮派生用过的那条）。
const FIRST_HOLDERS: usize = 8;

#[test]
fn derivations_on_changing_threads_do_not_multiply_resident_memory() {
    let mut probe = Command::new(std::env::current_exe().expect("取得测试二进制路径"));
    probe
        .args([
            "--exact",
            "coordinator_tests::memory::resident_memory_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PROBE, "1");
    // glibc 的 malloc 调参会改变 arena 行为（比如 MALLOC_ARENA_MAX=1 会让回归也量不出来），探针一律不继承。
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|key| key.starts_with("MALLOC_") || key == "GLIBC_TUNABLES")
        {
            probe.env_remove(key);
        }
    }
    let output = probe.output().expect("启动探针子进程");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "探针失败：\n{stdout}\n{stderr}");
    // libtest 会把探针的第一行输出接在「test ... 」后面，所以按「键=」切分，不按行首匹配。
    let reading = |key: &str| -> u64 {
        stdout
            .lines()
            .find_map(|line| line.split_once(&format!("{key}=")))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("探针没有报告 {key}：\n{stdout}\n{stderr}"))
    };
    let corpus = reading("CORPUS_KIB");
    let baseline = reading("BASELINE_KIB");
    let after = reading("AFTER_KIB");
    let grown = after.saturating_sub(baseline);
    // 派生都在专用线程上时，换线程不该带来增长。上限 3 × 正文 + 8 MiB（约 15 MiB）小于一次派生在
    // 新 arena 上的占用（debug 构建约 16 倍正文、37 MiB），只要有一次派生落到别的线程上就超限；
    // 8 MiB 留给占位线程的栈与它们各自的小 arena（改码后实测：debug 共增长 240 KiB，发布版 704 KiB）。
    let bound = 3 * corpus + 8 * 1024;
    eprintln!(
        "canonical 派生内存自证：正文 {corpus} KiB；预热后 RssAnon {baseline} KiB；{ROUNDS} 轮换线程刷新后 \
         {after} KiB；增长 {grown} KiB，上限 {bound} KiB"
    );
    assert!(
        grown <= bound,
        "{ROUNDS} 轮换线程刷新让常驻匿名内存涨了 {grown} KiB（上限 {bound} KiB）：\
         整库派生落在不同线程上，各自的 malloc arena 在 mlockall 下都退不回去"
    );
}

/// 探针：在一次性子进程里与生产一样加固，再起真实协调器、换线程刷新并报告读数。
#[test]
fn resident_memory_probe() {
    if std::env::var_os(PROBE).is_none() {
        return;
    }
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
        .block_on(probe());
}

async fn probe() {
    let notes = (0..NOTES)
        .map(|note| (format!("n{note:04}.md"), note_body(note)))
        .collect::<Vec<_>>();
    let corpus: usize = notes.iter().map(|(_, body)| body.len()).sum();
    let fixture = Fixture::with_notes(&notes);
    drop(notes);
    let workbench = fixture.start(false).await;
    // 预热：顺序刷新几次，让一次派生的峰值和在用的一代视图落进基线；之后的增长只来自换线程。
    for _ in 0..3 {
        bounded(workbench.refresh()).await.expect("预热刷新");
    }
    let baseline = rss_anon_kib();
    for round in 0..ROUNDS {
        let holders = bounded(Holders::occupy(FIRST_HOLDERS + round)).await;
        bounded(workbench.refresh()).await.expect("换线程后的刷新");
        bounded(holders.release()).await;
    }
    let after = rss_anon_kib();
    println!("CORPUS_KIB={}", corpus / 1024);
    println!("BASELINE_KIB={baseline}");
    println!("AFTER_KIB={after}");
    bounded(workbench.shutdown()).await.expect("关停协调器");
}

/// 探针里的每次等待都有时限：派生线程挂住时按时失败退出，不把父进程那边的整轮测试拖死。
async fn bounded<T>(work: impl Future<Output = T>) -> T {
    tokio::time::timeout(super::TIMEOUT, work)
        .await
        .expect("探针里的等待超过时限，按挂住处理")
}

/// 占住阻塞线程池的别的阻塞任务，生产上对应黑板维护、Bridge 轮次与 HTTP 查询。
struct Holders {
    release: tokio::sync::OwnedRwLockWriteGuard<()>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Holders {
    /// 起 `count` 个阻塞任务并等它们都真正开始执行：各占一条线程，直到 [`Holders::release`]。
    async fn occupy(count: usize) -> Self {
        let gate = Arc::new(tokio::sync::RwLock::new(()));
        let release = Arc::clone(&gate).write_owned().await;
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let tasks = (0..count)
            .map(|_| {
                let gate = Arc::clone(&gate);
                let started = started.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = started.send(());
                    drop(gate.blocking_read());
                })
            })
            .collect();
        for _ in 0..count {
            ready.recv().await.expect("占位任务开始执行");
        }
        Self { release, tasks }
    }

    async fn release(self) {
        drop(self.release);
        for task in self.tasks {
            task.await.expect("占位任务结束");
        }
    }
}

/// 一篇虚构笔记：标题、小节和链到别篇的 wikilink，让索引建出与真实笔记同类的结构。
fn note_body(note: usize) -> Vec<u8> {
    let mut body = format!("# 虚构笔记 {note}\n\n## 小节\n\n");
    let mut line = 0;
    while body.len() < NOTE_BYTES {
        body.push_str(&format!(
            "- 第 {line} 行，虚构词 w{note}x{line}，见 [[n{:04}]]，不是秘密。\n",
            (note + line) % NOTES
        ));
        line += 1;
    }
    body.into_bytes()
}

/// 本进程的常驻匿名内存（KiB）：锁定后释放的堆页仍算在这里，cgroup 记账看的也是它。
fn rss_anon_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .expect("读取 /proc/self/status")
        .lines()
        .find_map(|line| line.strip_prefix("RssAnon:"))
        .and_then(|value| value.split_whitespace().next()?.parse().ok())
        .expect("/proc/self/status 里有 RssAnon")
}
