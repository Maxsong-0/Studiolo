//! rmcp 的实际 SID 与外壳预占同生共灭；取消初始化不会遗留无主 worker。
use super::{binding::Binding, pool::Pool, worker::Worker};
use futures_core::Stream;
use rmcp::transport::Transport as _;
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::streamable_http_server::session::{
        ServerSseMessage, SessionId, SessionManager,
        local::{LocalSessionManager, LocalSessionManagerError},
    },
};
use std::{sync::Arc, time::Instant};

tokio::task_local! { pub(super) static REQUEST_BINDING: Arc<Binding>; }

/// create_session 正在等 SDK 锁时，清理也不能把该预占当作“尚无 worker”提前释放。
struct Creation(Arc<Binding>);
impl Drop for Creation {
    fn drop(&mut self) {
        self.0.creation_finished();
    }
}

#[derive(Clone)]
pub(super) struct Manager {
    pub inner: Arc<LocalSessionManager>,
    pub pool: Arc<Pool>,
}

impl Manager {
    /// 先标记关闭，再启动不可被请求取消的清理；容量直到 close 完成才回收。
    pub fn discard(&self, binding: Arc<Binding>) {
        if !binding.start_closing() {
            return;
        }
        let manager = self.clone();
        tokio::spawn(async move {
            manager.finish_close(binding).await;
        });
    }

    /// 关停时关掉全部会话：每个会话名下的 SSE 流随即结束，客户端开着的 GET 流也一样（BUG-033）。
    /// 关停开始后门禁不再预占新会话，所以一轮就够。
    pub fn close_all(&self) {
        for binding in self.pool.all() {
            self.discard(binding);
        }
    }

    async fn finish_close(&self, binding: Arc<Binding>) {
        binding.wait_creation().await;
        if let Some(id) = binding.session.get() {
            let _ = self.inner.close_session(id).await;
            binding.wait_worker().await;
        }
        self.pool.release(&binding);
    }
}

impl SessionManager for Manager {
    type Error = LocalSessionManagerError;
    type Transport = Worker;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let binding = REQUEST_BINDING
            .try_with(Arc::clone)
            .map_err(|_| LocalSessionManagerError::SessionNotFound("unbound".into()))?;
        if !binding.start_worker() {
            return Err(LocalSessionManagerError::SessionNotFound(
                "already-bound".into(),
            ));
        }
        let _creation = Creation(binding.clone());
        let (id, mut transport) = self.inner.create_session().await?;
        // create_session 返回后到绑定 SID 之间没有 await，取消不能遗漏刚创建的 worker。
        if binding.session.set(id.clone()).is_err() {
            let _ = self.inner.close_session(&id).await;
            let _ = transport.close().await;
            return Err(LocalSessionManagerError::SessionNotFound(id));
        }
        Ok((id, Worker::new(transport, binding)))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        let binding = self
            .pool
            .find(id)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let ClientJsonRpcMessage::Request(request) = &message else {
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        };
        if !binding.prepare_initialize(request.id.clone()) {
            return Err(LocalSessionManagerError::SessionNotFound(id.clone()));
        }
        let response = self.inner.initialize_session(id, message).await?;
        if matches!(&response, ServerJsonRpcMessage::Response(_))
            && self
                .pool
                .find(id)
                .is_some_and(|binding| binding.initialize(Instant::now()))
        {
            return Ok(response);
        }
        Err(LocalSessionManagerError::SessionNotFound(id.clone()))
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        if !self
            .pool
            .find(id)
            .is_some_and(|binding| binding.live(Instant::now()))
        {
            return Ok(false);
        }
        self.inner.has_session(id).await
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        let Some(binding) = self.pool.find(id) else {
            return self.inner.close_session(id).await;
        };
        if binding.start_closing() {
            let manager = self.clone();
            let _ = tokio::spawn(async move {
                manager.finish_close(binding).await;
            })
            .await;
        }
        Ok(())
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let binding = self
            .pool
            .find(id)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let stream = self.inner.create_stream(id, message).await?;
        binding
            .streams
            .wrap(stream)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        self.inner.accept_message(id, message).await
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let binding = self
            .pool
            .find(id)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let stream = self.inner.create_standalone_stream(id).await?;
        binding
            .streams
            .wrap(stream)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        let binding = self
            .pool
            .find(id)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))?;
        let stream = self.inner.resume(id, last_event_id).await?;
        binding
            .streams
            .wrap(stream)
            .ok_or_else(|| LocalSessionManagerError::SessionNotFound(id.clone()))
    }
}

pub(super) struct Reservation {
    pub binding: Arc<Binding>,
    manager: Arc<Manager>,
    published: bool,
}

impl Reservation {
    pub fn new(binding: Arc<Binding>, manager: Arc<Manager>) -> Self {
        Self {
            binding,
            manager,
            published: false,
        }
    }

    pub fn publish(&mut self) {
        self.published = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.published {
            self.manager.discard(self.binding.clone());
        }
    }
}
