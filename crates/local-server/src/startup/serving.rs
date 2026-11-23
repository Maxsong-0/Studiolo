//! 最后复核并选择一个监听器；因子模式不隐式留下TCP旁路。

use super::{PreparedServer, StartupError, backup_input, canonical_input, checkpoint_input};
use std::{future::Future, net::Ipv4Addr, sync::Arc};
use studiolo_local_server::BoundUnixIngress;
use tokio::signal::unix::{Signal, SignalKind, signal};

impl PreparedServer {
    pub(super) fn revalidate(&self) -> Result<(), StartupError> {
        if let Some(repository) = &self.repository {
            repository
                .revalidate()
                .map_err(|_| StartupError::ReadRoots)?;
        }
        if let Some(roots) = &self.roots {
            roots
                .revalidate()
                .map_err(|_| StartupError::Configuration)?;
        }
        if let Some(canonical) = &self.canonical {
            canonical
                .root_identity()
                .map_err(|_| StartupError::ReadRoots)?;
        }
        if let Some(ingress) = &self.board_ingress {
            ingress.revalidate().map_err(|_| StartupError::Ingress)?;
        }
        if let Some(ingress) = &self.ingress {
            ingress.revalidate().map_err(|_| StartupError::Ingress)?;
        }
        if let Some(workbench) = &self.workbench {
            workbench.revalidate().map_err(|_| StartupError::Writer)?;
        }
        Ok(())
    }

    pub(super) async fn bind(
        self,
    ) -> Result<(tokio::net::TcpListener, axum::Router), StartupError> {
        if self.ingress.is_some() {
            return Err(StartupError::Configuration);
        }
        self.revalidate()?;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, self.port))
            .await
            .map_err(|_| StartupError::Listen)?;
        Ok((
            listener,
            checkpoint_input::attach(self.app, self.checkpoint),
        ))
    }

    pub(super) fn bind_unix(self) -> Result<(BoundUnixIngress, axum::Router), StartupError> {
        self.revalidate()?;
        let listener = self
            .ingress
            .ok_or(StartupError::Configuration)?
            .bind()
            .map_err(|_| StartupError::Ingress)?;
        Ok((
            listener,
            checkpoint_input::attach(self.app, self.checkpoint),
        ))
    }

    pub(crate) async fn serve(self) -> Result<(), StartupError> {
        if self.ingress.is_some() || self.workbench.is_some() {
            let shutdown = match Shutdown::new() {
                Ok(shutdown) => shutdown,
                Err(error) => {
                    canonical_input::shutdown(self.workbench.as_ref()).await?;
                    return Err(error);
                }
            };
            return self.serve_until(shutdown.wait()).await;
        }
        let (port, mode) = (self.port, self.mode);
        let (listener, app) = self.bind().await?;
        println!("Studiolo：http://127.0.0.1:{port}（{mode}；按 Ctrl-C 停止）");
        axum::serve(listener, app)
            .await
            .map_err(|_| StartupError::Serve)
    }

    // 私有组合Seam：生产只传真实信号；测试使用有界oneshot，不向进程发送信号。
    pub(super) async fn serve_until<F>(mut self, shutdown: F) -> Result<(), StartupError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let workbench = self.workbench.as_ref().map(Arc::clone);
        let stopping = workbench.as_ref().map(Arc::clone);
        // 备份 socket 配了就必须起得来；起不来先关停写者，再带着入口错误返回。
        let backup = match self
            .backup
            .take()
            .map(backup_input::Listening::start)
            .transpose()
        {
            Ok(listening) => listening,
            Err(error) => {
                canonical_input::shutdown(workbench.as_ref()).await?;
                return Err(error);
            }
        };
        // 退出信号或 board 一侧报故障都触发关停，走同一条收尾链；收尾链第一步停备份（见 `drain`）。
        let (board_fault, board_faulted) = tokio::sync::oneshot::channel();
        let shutdown = shutdown_requested(shutdown, board_faulted);
        let stop_backup = async move {
            if let Some(backup) = &backup {
                backup.shutdown().await;
            }
        };
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let driver = self
            .sync_plane
            .take()
            .map(|plane| tokio::spawn(plane.drive_until(stopped)));
        let (http_stop, http_stopped) = tokio::sync::oneshot::channel();
        let server = self.serve_bound_until(
            async move {
                let _ = http_stopped.await;
            },
            board_fault,
        );
        let result = drain(
            server,
            shutdown,
            stop_backup,
            driver,
            stop,
            http_stop,
            move || {
                if let Some(workbench) = stopping {
                    workbench.stop();
                }
            },
            |line| eprintln!("{line}"),
        )
        .await;
        canonical_input::shutdown(workbench.as_ref()).await?;
        result
    }

    /// 有 board 时连同租约巡检、board 监听一起服务。board 一侧先结束时经 `board_fault` 报给外层，
    /// 由外层照关停走完整收尾链；内层照旧等 `shutdown` 送来再排空，不在这里被丢弃。
    async fn serve_bound_until<F>(
        mut self,
        shutdown: F,
        board_fault: tokio::sync::oneshot::Sender<()>,
    ) -> Result<(), StartupError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if let (Some(board), Some(plane)) = (self.board_ingress.take(), self.board_plane.take()) {
            let runner = self.runner_plane.take();
            let listener = board.bind().map_err(|_| StartupError::Ingress)?;
            if let Some(runner) = &runner {
                runner.serve().map_err(|_| StartupError::Serve)?;
            }
            let app = studiolo_local_server::board::private_board_router(plane.clone());
            let (stop, mut stopped) = tokio::sync::watch::channel(false);
            let maintenance =
                tokio::spawn(plane.maintain_until_with_runner(stopped.clone(), runner.clone()));
            let worker = tokio::spawn(async move {
                listener
                    .serve_shutdown(app, async move {
                        let _ = stopped.changed().await;
                    })
                    .await
            });
            let mut result = settle_board(
                self.serve_listener_until(shutdown),
                maintenance,
                worker,
                stop,
                board_fault,
                |line| eprintln!("{line}"),
            )
            .await;
            if let Some(runner) = runner
                && !matches!(
                    tokio::task::spawn_blocking(move || runner.shutdown()).await,
                    Ok(Ok(()))
                )
            {
                result = Err(StartupError::Serve);
            }
            return result;
        }
        self.serve_listener_until(shutdown).await
    }

    /// 内层服务：可信 Unix 入口或本机 TCP；`shutdown` 一到就停止接新连接，在途请求排空后才返回。
    async fn serve_listener_until<F>(self, shutdown: F) -> Result<(), StartupError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if self.ingress.is_some() {
            let (listener, app) = self.bind_unix()?;
            println!("Studiolo：可信Unix入口（Access、应用会话及二次认证；按 Ctrl-C 停止）");
            return listener
                .serve_shutdown(app, shutdown)
                .await
                .map_err(|_| StartupError::Serve);
        }
        let (port, mode) = (self.port, self.mode);
        let (listener, app) = self.bind().await?;
        println!("Studiolo：http://127.0.0.1:{port}（{mode}；按 Ctrl-C 停止）");
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|_| StartupError::Serve)
    }
}

/// board 三方（内层服务、租约巡检、board 监听）谁先结束都走同一条收尾：通知另外两方停下，
/// 再收回没赢 `select!` 的句柄、逐个检查结果；任何一方出错，结果就是错误。
/// board 一侧先结束、收尾时出错，各经 `log` 打一行，只带固定类别（BUG-050：169 上人侧退出时
/// 日志里只有「HTTP服务已停止」，看不出是租约巡检先停的、为什么停）。
///
/// 内层服务手里有在途请求，不能随 `select!` 丢掉：axum 的连接各自在后台任务里跑，丢掉外层
/// future 就没人等它们排空，外层随后等写者收尾、进程退出时请求可能还在跑、被截断。board 一侧
/// 先结束时把故障经 `fault` 报给外层，外层照关停走完整收尾链（停备份、等 Bridge、停刷新、送来
/// 内层的关停信号），这里一直等到内层排空才返回。
async fn settle_board<S, M, W, L>(
    inner: S,
    mut maintenance: tokio::task::JoinHandle<Result<(), M>>,
    mut worker: tokio::task::JoinHandle<Result<(), W>>,
    stop: tokio::sync::watch::Sender<bool>,
    fault: tokio::sync::oneshot::Sender<()>,
    log: L,
) -> Result<(), StartupError>
where
    S: Future<Output = Result<(), StartupError>>,
    M: std::fmt::Display,
    L: Fn(String),
{
    #[derive(PartialEq, Eq)]
    enum First {
        Inner,
        Maintenance,
        Worker,
    }
    let mut inner = std::pin::pin!(inner);
    let (mut result, first) = tokio::select! {
        result = &mut inner => (result, First::Inner),
        ended = &mut maintenance => {
            log(format!("看板一侧先结束：租约巡检（{}）", maintenance_ending(&ended)));
            (Err(StartupError::Serve), First::Maintenance)
        }
        ended = &mut worker => {
            log(format!("看板一侧先结束：看板监听（{}）", task_ending(&ended)));
            (Err(StartupError::Serve), First::Worker)
        }
    };
    let _ = stop.send(true);
    if first != First::Inner {
        // 外层若已因退出信号开始收尾，接收端就已经不在了，发不出去也无妨：内层的关停信号照样会来。
        let _ = fault.send(());
        // 结果已经是错误，内层自己的结果不再改写它；但必须等它排空在途请求。
        let _ = inner.await;
    }
    // 只跳过赢了 select 的那个（结果已取走，再 poll 会 panic）；不能用 `is_finished()` 判断：
    // 它分不清「已取走」和「结束了还没取」，同时就绪或收尾途中出的错会被它跳过吞掉。
    if first != First::Worker {
        let ended = worker.await;
        if !matches!(ended, Ok(Ok(()))) {
            log(format!("看板收尾出错：看板监听（{}）", task_ending(&ended)));
            result = Err(StartupError::Serve);
        }
    }
    if first != First::Maintenance {
        let ended = maintenance.await;
        if !matches!(ended, Ok(Ok(()))) {
            log(format!(
                "看板收尾出错：租约巡检（{}）",
                maintenance_ending(&ended)
            ));
            result = Err(StartupError::Serve);
        }
    }
    result
}

/// 租约巡检怎么结束的：巡检自己的原因已是固定类别（`MaintenanceFault`），原样带上。
fn maintenance_ending<M: std::fmt::Display>(
    ended: &Result<Result<(), M>, tokio::task::JoinError>,
) -> String {
    match ended {
        Ok(Ok(())) => "正常结束".to_owned(),
        Ok(Err(fault)) => fault.to_string(),
        Err(_) => "异常退出".to_owned(),
    }
}

/// 看板监听、同步驱动怎么结束的：底层错误可能带路径，只记类别。
fn task_ending<E>(ended: &Result<Result<(), E>, tokio::task::JoinError>) -> &'static str {
    match ended {
        Ok(Ok(())) => "正常结束",
        Ok(Err(_)) => "出错",
        Err(_) => "异常退出",
    }
}

/// 关停从这里开始：退出信号或 board 一侧报的故障，先到的为准。发送端没报就丢下了
/// （没有 board，或者 board 正常收尾）不算故障，那一支停用，只等退出信号。
async fn shutdown_requested<F>(signal: F, board_faulted: tokio::sync::oneshot::Receiver<()>)
where
    F: Future<Output = ()>,
{
    tokio::select! {
        () = signal => {}
        Ok(()) = board_faulted => {}
    }
}

/// 关闭链只走一遍：先停备份（在途的封存握着全部 owner 的冻结屏障，要等它放开才能动写者）；
/// 再通知并等待 Bridge 安全收尾，写者仍可用；再停止刷新并排空 HTTP。
/// 退出信号、board 一侧报故障（都经 `shutdown` 进来）、监听失败、驱动异常退出，哪一支先到都走完整收尾，
/// 不能丢弃尚在处理请求的服务 future。
///
/// 停备份不放进 `select!`：放在关停那一支里，别的分支先到就会把它中途丢掉，备份已经把要等的
/// 句柄取走了，事后再调也等不到在途的连接和封存。
///
/// 关停从哪一支开始、收尾时同步驱动出没出错，各经 `log` 打一行固定类别（BUG-050）。
#[allow(clippy::too_many_arguments)]
async fn drain<S, F, B, W, L>(
    server: S,
    shutdown: F,
    stop_backup: B,
    mut driver: Option<tokio::task::JoinHandle<Result<(), String>>>,
    stop: tokio::sync::watch::Sender<bool>,
    http_stop: tokio::sync::oneshot::Sender<()>,
    stop_workbench: W,
    log: L,
) -> Result<(), StartupError>
where
    S: Future<Output = Result<(), StartupError>>,
    F: Future<Output = ()>,
    B: Future<Output = ()>,
    W: FnOnce(),
    L: Fn(String),
{
    let mut server = Box::pin(server);
    let (mut result, mut served, joined) = tokio::select! {
        () = shutdown => {
            log("关停起因：退出信号或看板一侧先结束".to_owned());
            (Ok(()), false, false)
        }
        result = &mut server => {
            let how = match &result {
                Ok(()) => "正常结束".to_owned(),
                Err(error) => error.to_string(),
            };
            log(format!("关停起因：HTTP服务先结束（{how}）"));
            (result, true, false)
        }
        ended = async {
            match driver.as_mut() {
                Some(driver) => driver.await,
                None => std::future::pending().await,
            }
        } => {
            log(format!("关停起因：同步驱动先结束（{}）", task_ending(&ended)));
            (Err(StartupError::Serve), false, true)
        }
    };
    // 停备份期间服务照旧在跑、能接新连接；服务这时先结束只记下结果，不打断停备份。
    let mut stop_backup = std::pin::pin!(stop_backup);
    let mut backup_stopped = false;
    if !served {
        tokio::select! {
            () = &mut stop_backup => backup_stopped = true,
            early = &mut server => {
                served = true;
                if let Err(error) = early {
                    result = Err(error);
                }
            }
        }
    }
    if !backup_stopped {
        stop_backup.await;
    }
    let _ = stop.send(true);
    if !joined && let Some(driver) = driver {
        let ended = driver.await;
        if !matches!(ended, Ok(Ok(()))) {
            log(format!("关停收尾出错：同步驱动（{}）", task_ending(&ended)));
            result = Err(StartupError::Serve);
        }
    }
    stop_workbench();
    let _ = http_stop.send(());
    if !served && let Err(error) = server.await {
        result = Err(error);
    }
    result
}

struct Shutdown {
    interrupt: Signal,
    terminate: Signal,
}

impl Shutdown {
    fn new() -> Result<Self, StartupError> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt()).map_err(|_| StartupError::Process)?,
            terminate: signal(SignalKind::terminate()).map_err(|_| StartupError::Process)?,
        })
    }

    async fn wait(mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.terminate.recv() => {},
        }
    }
}

#[cfg(test)]
#[path = "../../tests/startup/shutdown_order.rs"]
mod shutdown_order;

#[cfg(test)]
#[path = "../../tests/startup/board_settle.rs"]
mod board_settle;

#[cfg(test)]
#[path = "../../tests/startup/board_drain.rs"]
mod board_drain;
