//! 整库派生的专用线程（T-186，vault BUG-035）。
//!
//! 本进程按秘密驻留面契约 `mlockall` 了整个地址空间（`startup.rs`），glibc 释放内存时用的
//! `madvise(MADV_DONTNEED)` 对锁定页一律返回 EINVAL：释放掉的页留在分配它的 malloc arena 里，
//! 常驻集只涨不退；并发分配时 glibc 又把线程分到不同的 arena（上限 8 × 核数）。整库派生——
//! canonical 协调器的完整捕获加重建工作台模型、只读副本的重扫加建模——每次临时分配都随正文总量
//! 增长。原先它跑在 tokio 阻塞线程池上：池里线程空闲 10 秒就退出、没有空闲线程就新开，而 canonical
//! 每 30 秒例行检查一次，常和黑板维护（每 15 秒）、Bridge 轮次、HTTP 查询同时占着池里的线程，
//! 派生于是轮流落在不同线程、不同 arena 上，每个 arena 都停在一次派生的峰值。2026-10-16 生产 169
//! 上这样的 arena 堆五段合计约 300 MiB，单元读数逼近 `MemoryMax=512M`。
//!
//! 所以整库派生只交给这一条线程串行做：协调器作业本来就至多一次在跑，副本刷新本来就由闸门合流，
//! 吞吐不变。分配都落在同一个 arena 里，常驻上界是「在用的一代视图 + 一次派生」，不再随做过派生的
//! 线程数相乘，也不随运行时长累加。整库建模（`view::build`，`view::load` 经由它）要
//! [`OnDeriveThread`] 凭证，只有这条线程上的任务拿得到：在别的线程上做整库建模编译不过。
//!
//! 这条线程不归 tokio 运行时管。持有投递口的协调器或副本状态放下后，它做完已投递的活就退出；
//! 进程退出时随之结束。

use std::{
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
};

use crate::LocalError;

/// 线程名：`ps -L`、`top -H` 里认得出它（Linux 线程名上限 15 字节）。
pub(crate) const THREAD_NAME: &str = "studiolo-derive";
/// 副本模式的启动建模原先在主线程上做（默认 8 MiB 栈），这里给同样大的栈；
/// `MCL_ONFAULT` 只锁碰到的栈页，不多占内存。
const STACK_BYTES: usize = 8 * 1024 * 1024;

/// 只在专用派生线程上存在的凭证，整库建模的函数都要它。
///
/// 它不能跨线程（`!Send`、`!Sync`），只以借用的形式交给任务，带不出这条线程。
pub(crate) struct OnDeriveThread(PhantomData<*const ()>);

type Job = Box<dyn FnOnce(&OnDeriveThread) + Send>;

/// 专用派生线程的投递口；放下后（随协调器或副本状态析构），线程做完已投递的活就退出。
pub(crate) struct Deriver {
    jobs: mpsc::Sender<Job>,
    /// 专用线程的标识：[`Deriver::run_blocking`] 据此拒绝在它自己的任务里同步等它。
    thread: thread::ThreadId,
    /// 测试用：置位后投递一律像线程已不在那样失败。
    #[cfg(test)]
    lost: std::sync::atomic::AtomicBool,
}

impl Deriver {
    /// 起专用线程；起不来就不提供视图。
    pub(crate) fn start() -> Result<Self, LocalError> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let worker = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .stack_size(STACK_BYTES)
            .spawn(move || {
                let on = OnDeriveThread(PhantomData);
                // 一次任务 panic 只让投它的那一方失败，线程接着做后面的派生，
                // 与原先阻塞池里单个任务 panic 的影响面相同。
                for job in queue {
                    let _ = catch_unwind(AssertUnwindSafe(|| job(&on)));
                }
            })
            .map_err(|_| LocalError::Unavailable)?;
        Ok(Self {
            jobs,
            thread: worker.thread().id(),
            #[cfg(test)]
            lost: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// 投递一件派生，不等结果；任务自己负责结算，含 panic 时的收尾。
    pub(crate) fn submit<F>(&self, job: F) -> Result<(), LocalError>
    where
        F: FnOnce(&OnDeriveThread) + Send + 'static,
    {
        #[cfg(test)]
        if self.lost.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(LocalError::Unavailable);
        }
        self.jobs
            .send(Box::new(job))
            .map_err(|_| LocalError::Unavailable)
    }

    /// 测试用：模拟专用线程已不在，此后的投递都失败。
    #[cfg(test)]
    pub(crate) fn lose(&self) {
        self.lost.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 投递一件派生，返回等结果的 future。投递在调用时就完成，不等 future 被轮询；
    /// 任务 panic 或线程已不在时得到 `Err`，相当于原先阻塞任务的 `JoinError`。
    pub(crate) fn run<T, F>(
        &self,
        work: F,
    ) -> impl Future<Output = Result<T, LocalError>> + Send + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&OnDeriveThread) -> T + Send + 'static,
    {
        let (reply, result) = tokio::sync::oneshot::channel();
        let submitted = self.submit(move |on| {
            let _ = reply.send(work(on));
        });
        async move {
            submitted?;
            result.await.map_err(|_| LocalError::Unavailable)
        }
    }

    /// 带着查询许可投递：许可随作业结束才归还，不随等结果的 future。客户端断开只丢掉 future，
    /// 已投递的作业照样在这条线程上跑完；额度在那之前不能放给新请求，否则并发上限对它不成立
    /// （与 `queries::run_query` 同一口径）。
    pub(crate) fn run_leased<T, F>(
        &self,
        permit: tokio::sync::OwnedSemaphorePermit,
        work: F,
    ) -> impl Future<Output = Result<T, LocalError>> + Send + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&OnDeriveThread) -> T + Send + 'static,
    {
        self.run(move |on| {
            let _permit = permit;
            work(on)
        })
    }

    /// 同步装配（只读副本的启动建模）用：阻塞当前线程等结果。在专用线程自己的任务里调用，
    /// 等的就是自己、永远等不到，所以直接拒绝。
    pub(crate) fn run_blocking<T, F>(&self, work: F) -> Result<T, LocalError>
    where
        T: Send + 'static,
        F: FnOnce(&OnDeriveThread) -> T + Send + 'static,
    {
        if thread::current().id() == self.thread {
            return Err(LocalError::Unavailable);
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.submit(move |on| {
            let _ = reply.send(work(on));
        })?;
        result.recv().map_err(|_| LocalError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[tokio::test]
    async fn a_leased_job_keeps_its_query_slot_until_it_finishes_even_if_the_caller_goes_away() {
        let deriver = Deriver::start().unwrap();
        let budget = Arc::new(tokio::sync::Semaphore::new(1));
        // 先占住派生线程，后面那件带许可的作业只能排队。
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        deriver
            .submit(move |_| {
                let _ = started_tx.send(());
                // 发送端因断言失败被丢弃时也退出，不留下卡住的线程。
                let _ = release_rx.recv();
            })
            .unwrap();
        started_rx.recv().unwrap();
        let permit = Arc::clone(&budget).try_acquire_owned().unwrap();
        // 客户端断开：等结果的 future 被丢弃，作业已经投递出去。
        drop(deriver.run_leased(permit, |_| 7));
        assert_eq!(budget.available_permits(), 0, "作业还没跑，额度不能先还");
        assert!(Arc::clone(&budget).try_acquire_owned().is_err());
        release_tx.send(()).unwrap();
        // 作业跑完、闭包析构时才还额度。
        let returned =
            tokio::time::timeout(Duration::from_secs(10), Arc::clone(&budget).acquire_owned())
                .await
                .expect("作业跑完后额度要还回来")
                .unwrap();
        drop(returned);
        assert_eq!(budget.available_permits(), 1);
    }
}
