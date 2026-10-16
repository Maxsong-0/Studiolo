//! 副本校验、重扫与首扫的专用线程（T-185，vault BUG-034）。
//!
//! 本进程按秘密驻留面契约 `mlockall` 了整个地址空间，glibc 释放内存时用的
//! `madvise(MADV_DONTNEED)` 对锁定页一律返回 EINVAL：释放掉的页留在分配它的 malloc arena 里，
//! 常驻集只涨不退。并发分配时 glibc 又会把线程分到不同的 arena（上限 8 × 核数）。
//! 托管副本模式每次内容请求都要整读、核对全部 Markdown，临时分配约三份正文；原先这件事跑在
//! tokio 阻塞线程池上，并发请求分到几条线程，常驻集就停在几份校验峰值。2026-10-15 生产副本
//! 1454 篇（14.4 MiB）时，几轮并发就把单元顶到 `MemoryMax=512M`，被 cgroup OOM 杀。
//!
//! 所以这些活都只交给这一条线程串行做：重扫本来就由 `scan_lock` 串行，吞吐不变。它们的分配都落在
//! 同一个 arena 里，常驻上界是「同时在用的几份代次 + 一次校验或建模」：请求拿到的代次用完即放，
//! 校验又是一个个出队，同时在用的代次只有寥寥几份；不再随做过校验的线程数相乘，也不随运行时长累加。
//!
//! 关停（T-184，BUG-033）：后台重扫由 `serve::run` 等它做完，请求触发的校验随请求排空做完；
//! 客户端已断开、没人等结果的那次不等，进程退出时随之结束。这条线程不归 tokio 运行时管，
//! `shutdown_timeout` 等不到它；它只读供给源，派生缓存写入是 SQLite 事务，中途结束不会写坏。

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
};

/// 线程名：`ps -L`、`top -H` 里认得出它。
const THREAD_NAME: &str = "studiolo-verify";
/// 首扫原先在主线程上做（默认 8 MiB 栈），这里给同样大的栈；`MCL_ONFAULT` 只锁碰到的栈页，不多占内存。
const STACK_BYTES: usize = 8 * 1024 * 1024;
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
            .name(THREAD_NAME.to_owned())
            .stack_size(STACK_BYTES)
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
            // 轮到它时调用方已经放弃（请求被取消、客户端断开、网关超时），就不再做这次整读校验：
            // 队列只有一条，白做的活会直接拖慢后面还在等的请求。
            if reply.is_closed() {
                return;
            }
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

#[cfg(test)]
#[path = "../tests/verifier_private/queue.rs"]
mod tests;
