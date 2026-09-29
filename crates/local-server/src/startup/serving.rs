//! 最后复核并选择一个监听器；因子模式不隐式留下TCP旁路。

use super::{PreparedServer, StartupError, canonical_input};
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
        Ok((listener, self.app))
    }

    pub(super) fn bind_unix(self) -> Result<(BoundUnixIngress, axum::Router), StartupError> {
        self.revalidate()?;
        let listener = self
            .ingress
            .ok_or(StartupError::Configuration)?
            .bind()
            .map_err(|_| StartupError::Ingress)?;
        Ok((listener, self.app))
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
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let driver = self
            .sync_plane
            .take()
            .map(|plane| tokio::spawn(plane.drive_until(stopped)));
        let (http_stop, http_stopped) = tokio::sync::oneshot::channel();
        let server = self.serve_bound_until(async move {
            let _ = http_stopped.await;
        });
        let result = drain(server, shutdown, driver, stop, http_stop, move || {
            if let Some(workbench) = stopping {
                workbench.stop();
            }
        })
        .await;
        canonical_input::shutdown(workbench.as_ref()).await?;
        result
    }

    async fn serve_bound_until<F>(mut self, shutdown: F) -> Result<(), StartupError>
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
            let mut maintenance =
                tokio::spawn(plane.maintain_until_with_runner(stopped.clone(), runner.clone()));
            let mut worker = tokio::spawn(async move {
                listener
                    .serve_shutdown(app, async move {
                        let _ = stopped.changed().await;
                    })
                    .await
            });
            let mut result = tokio::select! {
                result=Box::pin(self.serve_bound_until(shutdown))=>result,
                _=&mut maintenance=>Err(StartupError::Serve),
                _=&mut worker=>Err(StartupError::Serve),
            };
            let _ = stop.send(true);
            if !worker.is_finished() && !matches!(worker.await, Ok(Ok(()))) {
                result = Err(StartupError::Serve);
            }
            if !maintenance.is_finished() && !matches!(maintenance.await, Ok(Ok(()))) {
                result = Err(StartupError::Serve);
            }
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

/// 关闭链只走一遍：先通知并等待 Bridge 安全收尾，写者仍可用；再停止刷新并排空 HTTP。
/// 监听失败或驱动异常退出同样走完整收尾，不能丢弃尚在处理请求的服务 future。
async fn drain<S, F, W>(
    server: S,
    shutdown: F,
    mut driver: Option<tokio::task::JoinHandle<Result<(), String>>>,
    stop: tokio::sync::watch::Sender<bool>,
    http_stop: tokio::sync::oneshot::Sender<()>,
    stop_workbench: W,
) -> Result<(), StartupError>
where
    S: Future<Output = Result<(), StartupError>>,
    F: Future<Output = ()>,
    W: FnOnce(),
{
    let mut server = Box::pin(server);
    let (mut result, served, joined) = tokio::select! {
        () = shutdown => (Ok(()), false, false),
        result = &mut server => (result, true, false),
        _ = async {
            match driver.as_mut() {
                Some(driver) => { let _ = driver.await; }
                None => std::future::pending::<()>().await,
            }
        } => (Err(StartupError::Serve), false, true),
    };
    let _ = stop.send(true);
    if !joined
        && let Some(driver) = driver
        && !matches!(driver.await, Ok(Ok(())))
    {
        result = Err(StartupError::Serve);
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
