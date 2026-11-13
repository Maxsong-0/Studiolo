//! 托管副本逐请求整读校验的常驻内存上界（T-185，vault BUG-034）。
//!
//! 生产 agent 按秘密驻留面契约 `mlockall` 了整个地址空间，glibc 释放内存时用的
//! `madvise(MADV_DONTNEED)` 对锁定页一律返回 EINVAL：释放掉的页留在分配它的 malloc arena 里。
//! glibc 又给并发分配的每条线程各开一个 arena，所以校验分散到几条线程上，常驻集就停在几份
//! 「单次校验峰值」。2026-10-15 生产副本 1454 篇（14.4 MiB）时，并发请求把单元顶到
//! `MemoryMax=512M` 被 cgroup OOM 杀。
//!
//! `mlockall` 影响其后所有映射，不能加到共享的测试进程上（同 process-safety 的约定）：本测试二进制
//! 以探针身份重跑自己，子进程先像生产一样加固，再起真实 loopback HTTP 服务、打并发读请求，
//! 量自己的常驻匿名内存交给父进程判定。夹具只用虚构笔记，不含任何真实内容、token 或口令。
#![cfg(target_os = "linux")]

#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;
#[allow(dead_code)]
#[path = "http_text/support.rs"]
mod support;

use std::process::Command;

use replica_support::{ReplicaFixture, SECOND};
use studiolo_agent_server::options::ServeOptions;

/// 置位后本测试二进制作为探针运行；产品源码没有这个入口。
const PROBE: &str = "STUDIOLO_REPLICA_MEMORY_PROBE";
/// 虚构正文约 2.3 MiB：够让每次校验的临时分配远大于噪声，debug 构建下一次校验也只要零点几秒。
const NOTES: usize = 192;
const NOTE_BYTES: usize = 12 * 1024;
/// 与 HTTP 入口的并发供给上限一致（`http_text.rs` 的 `Semaphore::new(16)`）。
const CONCURRENCY: usize = 16;
const BURSTS: usize = 2;

#[test]
fn concurrent_reads_do_not_multiply_resident_memory() {
    let mut probe = Command::new(std::env::current_exe().expect("取得测试二进制路径"));
    probe
        .args([
            "--exact",
            "replica_memory_probe",
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
    // libtest 会把探针的第一行输出接在「test replica_memory_probe ... 」后面，所以按「键=」切分，不按行首匹配。
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
    // 一次校验临时要约三份正文（两轮扫描各读一份、再拼一份快照）。校验都在一条线程上串行做时，
    // 并发至多让旧代次多活一会儿，增长应在一次校验的量之内；8 MiB 留给连接缓冲与工作线程。
    let bound = 3 * corpus + 8 * 1024;
    eprintln!(
        "副本内存自证：正文 {corpus} KiB；预热后 RssAnon {baseline} KiB；{BURSTS} 轮 {CONCURRENCY} 路并发读后 \
         {after} KiB；增长 {grown} KiB，上限 {bound} KiB"
    );
    assert!(
        grown <= bound,
        "{BURSTS} 轮 {CONCURRENCY} 路并发读让常驻匿名内存涨了 {grown} KiB（上限 {bound} KiB）：\
         校验分散到多条线程，各自的 malloc arena 在 mlockall 下都退不回去"
    );
}

/// 探针：在一次性子进程里与生产一样加固，再起真实 HTTP 服务、打并发读请求并报告读数。
#[test]
fn replica_memory_probe() {
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
    let (replica, corpus) = corpus_replica();
    let server = support::spawn(ServeOptions {
        board_socket: None,
        index_cache: None,
        vault: None,
        replica_root: Some(replica.root.clone()),
        replica_max_age_secs: 300,
        allow: vec!["project".into()],
        data: replica.temp.path().join("agent-data"),
        policy: None,
        port: 0,
    })
    .await;
    let (session, _, _) = server.handshake().await;
    // 预热：先顺序读几次，让单次校验的峰值落进基线；之后的增长只来自并发。
    for note in 0..3 {
        let request = server.get(&route(note), Some(&session));
        assert_eq!(read(request).await.expect("预热读请求"), 200);
    }
    let baseline = rss_anon_kib();
    for _ in 0..BURSTS {
        let mut burst = tokio::task::JoinSet::new();
        for note in 0..CONCURRENCY {
            burst.spawn(read(server.get(&route(note), Some(&session))));
        }
        while let Some(done) = burst.join_next().await {
            assert_eq!(done.expect("并发读任务").expect("并发读请求"), 200);
        }
    }
    let after = rss_anon_kib();
    println!("CORPUS_KIB={}", corpus / 1024);
    println!("BASELINE_KIB={baseline}");
    println!("AFTER_KIB={after}");
}

/// 发布一份只含虚构笔记的副本，返回夹具与正文总字节数。
fn corpus_replica() -> (ReplicaFixture, usize) {
    let replica = ReplicaFixture::new();
    let index =
        "# Project\n\n| 项 | 当前值 |\n|---|---|\n| 当前版本 | memory |\n| 状态 | memory |\n";
    let notes = (0..NOTES)
        .map(|note| (format!("project/notes/n{note:04}.md"), note_body(note)))
        .collect::<Vec<_>>();
    let mut entries: Vec<(&str, Option<&[u8]>)> = vec![
        ("project/00-总览", None),
        ("project/00-总览/INDEX.md", Some(index.as_bytes())),
        ("project/notes", None),
    ];
    entries.extend(
        notes
            .iter()
            .map(|(path, body)| (path.as_str(), Some(body.as_slice()))),
    );
    replica.publish_entries(SECOND, replica.observed, &entries);
    let corpus = index.len() + notes.iter().map(|(_, body)| body.len()).sum::<usize>();
    (replica, corpus)
}

fn note_body(note: usize) -> Vec<u8> {
    let mut body = format!("# 虚构笔记 {note}\n\n");
    let mut line = 0;
    while body.len() < NOTE_BYTES {
        body.push_str(&format!(
            "- 第 {line} 行，虚构词 w{note}x{line}，不是秘密。\n"
        ));
        line += 1;
    }
    body.into_bytes()
}

fn route(note: usize) -> String {
    format!("/read/note?path=project%2Fnotes%2Fn{note:04}.md")
}

/// 发请求并读完响应体，像真实客户端一样让服务端把这次供给完整走完；返回状态码。
async fn read(request: reqwest::RequestBuilder) -> reqwest::Result<reqwest::StatusCode> {
    let response = request.send().await?;
    let status = response.status();
    response.bytes().await.map(|_| status)
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
