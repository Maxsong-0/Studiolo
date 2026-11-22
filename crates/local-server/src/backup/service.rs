//! 备份 socket 的接受循环和它的起停。一次只处理一个连接：对端只有一个 worker 账号，
//! 每个连接一问一答、十秒窗口，不需要并发。
use super::{
    listener::{self, ListenError},
    producer::BackupProducer,
};
use std::{
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{net::UnixListener, sync::watch, task::JoinHandle};

/// 单个连接的窗口；传输层自己还会再压到十秒以内。
const WINDOW: Duration = Duration::from_secs(10);
/// 接受失败后的退避，免得在文件句柄用尽这类情况下空转。
const BACKOFF: Duration = Duration::from_millis(200);
/// 只回收、不冻结的周期：监听起来时先回收一次，之后每隔这么久一次。过期的封存副本（含 Auth
/// 和审计）不能只靠 `schedule` 回收：定时器停了，它们就会一直留在盘上。
const RECLAIM_EVERY: Duration = Duration::from_secs(15 * 60);

/// 正在监听的备份 socket；停下时先收监听，再等在途的回收和封存做完。
pub struct BackupService {
    stop: watch::Sender<bool>,
    acceptor: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    /// 后台读账本的线程和叫停它的标志（`BackupProducer::catch_up`）。
    halt: Arc<AtomicBool>,
    chaser: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    producer: Arc<BackupProducer>,
}

impl BackupService {
    /// 在固定路径上建立监听并起接受循环；要在 Tokio 运行时里调用。
    pub fn start(producer: Arc<BackupProducer>, socket: &Path) -> Result<Self, ListenError> {
        let (_, reader_gid) = producer.identities();
        let (listener, claim) = listener::bind(socket, reader_gid)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| ListenError::Unavailable)?;
        let listener = UnixListener::from_std(listener).map_err(|_| ListenError::Unavailable)?;
        let (stop, stopped) = watch::channel(false);
        let reclaiming = Arc::clone(&producer);
        let serving = Arc::clone(&producer);
        let acceptor = tokio::spawn(async move {
            // 清理责任跟着接受循环走：循环结束才删 socket。
            let _claim = claim;
            accept_until_stopped(
                listener,
                stopped,
                RECLAIM_EVERY,
                move || reclaiming.reclaim(),
                // 对端身份、帧和期限都在连接里核；这里不看结果，失败的连接直接丢掉。
                move |stream| {
                    let _ = serving.serve_connection(stream, Instant::now() + WINDOW);
                },
            )
            .await;
        });
        // 监听一起来就在后台把账本读到末尾：队列进度只在内存里，不这样做，重启后要靠 worker 的请求
        // 一页一页推，账本长了头十几分钟答不上来（BUG-042）。
        let halt = Arc::new(AtomicBool::new(false));
        let (chasing, halted) = (Arc::clone(&producer), Arc::clone(&halt));
        let chaser = tokio::task::spawn_blocking(move || chasing.catch_up(&halted));
        Ok(Self {
            stop,
            acceptor: tokio::sync::Mutex::new(Some(acceptor)),
            halt,
            chaser: tokio::sync::Mutex::new(Some(chaser)),
            producer,
        })
    }

    /// 可以重复调用：叫停读账本的线程，停接受循环（顺带删掉自己的 socket），再等后台线程做完。
    pub async fn shutdown(&self) {
        // 先置标志：读账本的线程读完手上这一页就停，和下面等接受循环同时进行。
        self.halt.store(true, Ordering::Release);
        let _ = self.stop.send(true);
        if let Some(acceptor) = self.acceptor.lock().await.take() {
            let _ = acceptor.await;
        }
        if let Some(chaser) = self.chaser.lock().await.take() {
            let _ = chaser.await;
        }
        let producer = Arc::clone(&self.producer);
        let _ = tokio::task::spawn_blocking(move || producer.settle()).await;
    }
}

/// 接受循环：回收和每个连接都在后台线程里做完才回到循环顶上，一次只做一件。
///
/// 停止信号（发了值，或者发送端没了）一到，回到循环顶上就退出：backlog 里排着的连接不再接，
/// 到点的回收也不再起；停止信号到达时正在做的那一个连接或那一次回收照旧做完。
async fn accept_until_stopped<R, S>(
    listener: UnixListener,
    mut stopped: watch::Receiver<bool>,
    reclaim_every: Duration,
    reclaim: R,
    serve: S,
) where
    R: Fn() + Send + Sync + 'static,
    S: Fn(UnixStream) + Send + Sync + 'static,
{
    let (reclaim, serve) = (Arc::new(reclaim), Arc::new(serve));
    // 第一次 tick 立刻到：起来先回收一次。
    let mut tidy = tokio::time::interval(reclaim_every);
    tidy.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let stream = tokio::select! {
            // 按书写顺序看各分支：不加 `biased;` 时 Tokio 随机先看一个，停止信号已到、
            // 又排着连接或回收到点时，多半会先接连接或先回收，关停就要多等一轮。
            biased;
            // `changed()` 出错（发送端没了）也匹配 `_`，同样按停止处理。
            _ = stopped.changed() => break,
            _ = tidy.tick() => {
                let reclaim = Arc::clone(&reclaim);
                let _ = tokio::task::spawn_blocking(move || reclaim()).await;
                continue;
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(_) => {
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
            },
        };
        let Ok(stream) = stream.into_std() else {
            continue;
        };
        let serve = Arc::clone(&serve);
        let _ = tokio::task::spawn_blocking(move || serve(stream)).await;
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
