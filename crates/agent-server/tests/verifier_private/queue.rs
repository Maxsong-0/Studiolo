//! 专用校验线程的排队语义与源码守卫（T-185，vault BUG-034）：所有活落在同一条线程上、
//! 调用方放弃的排队任务不再做、一次 panic 只让投它的调用方失败；agent 源码里不许再出现
//! 把整读校验分到别的线程上的写法。
use super::{FAILED, THREAD_NAME, Verifier};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_job_runs_on_the_one_dedicated_thread() {
    let verifier = Arc::new(Verifier::start().expect("起专用线程"));
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let verifier = verifier.clone();
        jobs.spawn(async move {
            verifier
                .run(|| {
                    let current = thread::current();
                    (current.id(), current.name().map(str::to_owned))
                })
                .await
        });
    }
    let mut threads = HashSet::new();
    while let Some(done) = jobs.join_next().await {
        let (id, name) = done.expect("并发任务").expect("专用线程执行");
        assert_eq!(name.as_deref(), Some(THREAD_NAME));
        threads.insert(id);
    }
    assert_eq!(threads.len(), 1, "16 路并发也只能落在同一条线程上");
    let startup = verifier
        .run_blocking(|| thread::current().id())
        .expect("同步投递");
    assert!(threads.contains(&startup), "首扫与请求校验同一条线程");
}

#[tokio::test]
async fn queued_work_whose_caller_gave_up_is_skipped() {
    let verifier = Arc::new(Verifier::start().expect("起专用线程"));
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::channel::<()>();
    let busy = tokio::spawn({
        let verifier = verifier.clone();
        async move {
            verifier
                .run(move || {
                    let _ = started.send(());
                    let _ = gate.recv();
                })
                .await
        }
    });
    running.await.expect("占线任务开始执行");
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let abandoned = tokio::time::timeout(
        Duration::from_millis(50),
        verifier.run(move || flag.store(true, Ordering::SeqCst)),
    )
    .await;
    assert!(abandoned.is_err(), "线程被占着，排队的任务不该做完");
    release.send(()).expect("放开占线任务");
    busy.await.expect("占线任务").expect("占线任务的结果");
    // 队列先进先出：这一个做完时，前面被放弃的那个已经出队。
    verifier.run(|| ()).await.expect("后面的任务照常执行");
    assert!(
        !ran.load(Ordering::SeqCst),
        "调用方已放弃的排队任务不该再做整读校验"
    );
}

#[tokio::test]
async fn a_panicking_job_fails_only_its_own_caller() {
    let verifier = Verifier::start().expect("起专用线程");
    let failed = verifier
        .run(|| -> u8 { panic!("故意的 panic：只该让这一次调用失败") })
        .await;
    assert_eq!(failed, Err(FAILED.to_owned()));
    assert_eq!(verifier.run(|| 7_u8).await, Ok(7), "线程接着服务后面的请求");
}

/// 本进程 `mlockall`：阻塞池或临时线程上的大分配，释放后退不回去（BUG-034）。整读副本、重扫、
/// 首扫只交给专用校验线程，源码里不许再出现把活分到别的线程上的写法。确属小块阻塞 I/O 时，
/// 在这里加白名单并写明为什么不会累积常驻内存。
#[test]
fn agent_server_src_keeps_heavy_work_on_the_dedicated_thread() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0_usize;
    let mut violations = Vec::new();
    for path in rust_sources(&root) {
        let name = path
            .strip_prefix(&root)
            .expect("源码在 src 下")
            .to_string_lossy()
            .replace('\\', "/");
        // src 里以 `#[cfg(test)]` 引入的测试文件不进生产二进制，它们起线程造并发不算。
        if name.ends_with("/tests.rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let code = without_comments(&fs::read_to_string(&path).expect("读取本 crate 源码"));
        scanned += 1;
        for pattern in ["spawn_blocking", "block_in_place"] {
            if code.contains(pattern) {
                violations.push(format!("{name}：{pattern}"));
            }
        }
        if name != "verifier.rs" {
            for pattern in ["thread::spawn", "thread::Builder"] {
                if code.contains(pattern) {
                    violations.push(format!("{name}：{pattern}"));
                }
            }
        }
        // 重扫只在两处投给专用线程：请求供给（state.rs）与后台定时（refresh.rs）。
        if code.contains(".rescan(") && !matches!(name.as_str(), "state.rs" | "refresh.rs") {
            violations.push(format!("{name}：.rescan("));
        }
    }
    assert!(scanned >= 20, "源码目录读取不完整，守卫会漏扫");
    assert!(
        violations.is_empty(),
        "整读校验与大块临时分配只能交给 verifier 的专用线程（mlockall 后别的线程上的峰值退不回去）：{violations:?}"
    );
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

/// 去掉整行注释：文档里会提到这些写法的名字，不能被守卫误判为调用。
fn without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
