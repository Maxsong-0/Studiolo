//! 备份 socket 的接受循环和它的起停。一次只处理一个连接：对端只有一个 worker 账号，
//! 每个连接一问一答、十秒窗口，不需要并发。
use super::{
    listener::{self, ListenError},
    producer::BackupProducer,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinHandle};

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
        let listener =
            tokio::net::UnixListener::from_std(listener).map_err(|_| ListenError::Unavailable)?;
        let (stop, mut stopped) = watch::channel(false);
        let serving = Arc::clone(&producer);
        let acceptor = tokio::spawn(async move {
            // 清理责任跟着接受循环走：循环结束才删 socket。
            let _claim = claim;
            // 第一次 tick 立刻到：起来先回收一次。
            let mut tidy = tokio::time::interval(RECLAIM_EVERY);
            tidy.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let stream = tokio::select! {
                    _ = stopped.changed() => break,
                    _ = tidy.tick() => {
                        let producer = Arc::clone(&serving);
                        let _ = tokio::task::spawn_blocking(move || producer.reclaim()).await;
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
                let producer = Arc::clone(&serving);
                // 对端身份、帧和期限都在连接里核；这里不看结果，失败的连接直接丢掉。
                let _ = tokio::task::spawn_blocking(move || {
                    producer.serve_connection(stream, Instant::now() + WINDOW)
                })
                .await;
            }
        });
        Ok(Self {
            stop,
            acceptor: tokio::sync::Mutex::new(Some(acceptor)),
            producer,
        })
    }

    /// 可以重复调用：停接受循环（顺带删掉自己的 socket），再等后台线程做完。
    pub async fn shutdown(&self) {
        let _ = self.stop.send(true);
        if let Some(acceptor) = self.acceptor.lock().await.take() {
            let _ = acceptor.await;
        }
        let producer = Arc::clone(&self.producer);
        let _ = tokio::task::spawn_blocking(move || producer.settle()).await;
    }
}
