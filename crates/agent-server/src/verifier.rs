//! 副本校验、重扫与首扫的专用线程（T-185，vault BUG-034）。
//!
//! 本进程按秘密驻留面契约 `mlockall` 了整个地址空间，glibc 释放内存时用的
//! `madvise(MADV_DONTNEED)` 对锁定页一律返回 EINVAL：释放掉的页留在分配它的 malloc arena 里，
//! 常驻集只涨不退。glibc 又给并发分配的每条线程各开一个 arena（上限 8 × 核数）。
//! 托管副本模式每次内容请求都要整读、核对全部 Markdown，临时分配约三份正文；原先这件事跑在
//! tokio 阻塞线程池上，并发请求分到几条线程，常驻集就停在几份校验峰值。2026-10-15 生产副本
//! 1454 篇（14.4 MiB）时，几轮并发就把单元顶到 `MemoryMax=512M`，被 cgroup OOM 杀。
//!
//! 所以这些活都只交给这一条线程串行做：重扫本来就由 `scan_lock` 串行，吞吐不变；它们的分配
//! 都落在同一个 arena 里，常驻上界是「在用代次 + 一次校验或建模」，不随并发和运行时长累加。

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
};

/// 投递失败、任务 panic 时调用方拿到的固定原因；与原先阻塞池任务失败时的措辞一致。
const FAILED: &str = "副本验证任务失败";

type Job = Box<dyn FnOnce() + Send>;

/// 专用校验线程的投递口；它被放下（`AppState` 析构）后，线程做完已投递的活就退出。
pub(crate) struct Verifier {
    jobs: mpsc::Sender<Job>,
}

impl Verifier {
    /// 起专用线程；起不来就拒绝启动服务。
    pub(crate) fn start() -> Result<Self, String> {
        let (jobs, queue) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("studiolo-verify".to_owned())
            .spawn(move || {
                // 一次任务 panic 只让投它的那个调用方失败，线程接着服务后面的请求，
                // 与原先阻塞池里单个任务 panic 的影响面相同。
                for job in queue {
                    let _ = catch_unwind(AssertUnwindSafe(job));
                }
            })
            .map_err(|_| "副本校验线程无法启动".to_owned())?;
        Ok(Self { jobs })
    }

    /// 在专用线程上执行并异步等结果。
    pub(crate) async fn run<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (reply, result) = tokio::sync::oneshot::channel();
        self.submit(move || {
            let _ = reply.send(work());
        })?;
        result.await.map_err(|_| FAILED.to_owned())
    }

    /// 同步装配（启动首扫）用：阻塞当前线程等结果。不得在专用线程自己的任务里调用。
    pub(crate) fn run_blocking<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (reply, result) = mpsc::sync_channel(1);
        self.submit(move || {
            let _ = reply.send(work());
        })?;
        result.recv().map_err(|_| FAILED.to_owned())
    }

    fn submit(&self, job: impl FnOnce() + Send + 'static) -> Result<(), String> {
        self.jobs.send(Box::new(job)).map_err(|_| FAILED.to_owned())
    }
}
