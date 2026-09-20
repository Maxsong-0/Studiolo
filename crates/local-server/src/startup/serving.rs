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
    pub(super) async fn serve_until<F>(self, shutdown: F) -> Result<(), StartupError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        // bind消费PreparedServer后仍保留关闭owner；监听失败也走同一实际作业收尾。
        let workbench = self.workbench.as_ref().map(Arc::clone);
        let stopping = workbench.as_ref().map(Arc::clone);
        let signal = async move {
            shutdown.await;
            // HTTP drain之前停止新刷新并唤醒等待者，避免关闭与刷新彼此等待。
            if let Some(workbench) = stopping {
                workbench.stop();
            }
        };
        let result = self.serve_bound_until(signal).await;
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
