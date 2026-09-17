//! 有界代理独占 SDK worker，初始化 handler 未退出时也能关闭并确认实际 join。
use super::binding::Binding;
use rmcp::{
    RoleServer,
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::{
        Transport,
        streamable_http_server::session::{
            SessionManager,
            local::{LocalSessionManager, LocalSessionWorkerError},
        },
    },
};
use std::{pin::Pin, sync::Arc};
use tokio::sync::{mpsc, oneshot};

type Inner = <LocalSessionManager as SessionManager>::Transport;
type Error = LocalSessionWorkerError;
struct Sending {
    message: ServerJsonRpcMessage,
    result: oneshot::Sender<Result<(), Error>>,
}
type SendResult = (oneshot::Sender<Result<(), Error>>, Result<(), Error>);
struct Pending {
    initialize: bool,
    sending: Pin<Box<dyn Future<Output = SendResult> + Send>>,
}

async fn completed(pending: &mut Option<Pending>) -> SendResult {
    match pending {
        Some(pending) => pending.sending.as_mut().await,
        None => std::future::pending().await,
    }
}

pub(super) struct Worker {
    sending: mpsc::Sender<Sending>,
    receiving: mpsc::Receiver<ClientJsonRpcMessage>,
    binding: Arc<Binding>,
}

impl Worker {
    pub fn new(inner: Inner, binding: Arc<Binding>) -> Self {
        let (sending, requests) = mpsc::channel(16);
        let (received, receiving) = mpsc::channel(16);
        tokio::spawn(run(inner, binding.clone(), requests, received));
        Self {
            sending,
            receiving,
            binding,
        }
    }
}

async fn run(
    mut inner: Inner,
    binding: Arc<Binding>,
    mut requests: mpsc::Receiver<Sending>,
    received: mpsc::Sender<ClientJsonRpcMessage>,
) {
    let mut pending: Option<Pending> = None;
    let mut delivery = None;
    loop {
        tokio::select! {
            biased;
            _ = binding.wait_stop() => break,
            (responder, result) = completed(&mut pending) => {
                if pending.take().is_some_and(|sent| sent.initialize) {
                    binding.initialize_answered();
                }
                let _ = responder.send(result);
            }
            request = requests.recv(), if pending.is_none() => {
                let Some(request) = request else { break; };
                let initialize = binding.is_initialize_response(&request.message);
                let sending = inner.send(request.message);
                pending = Some(Pending { initialize, sending: Box::pin(async move {
                    (request.result, sending.await)
                }) });
            }
            permit = received.reserve(), if delivery.is_some() => {
                let Ok(permit) = permit else { break; };
                if let Some(message) = delivery.take() { permit.send(message); }
            }
            message = inner.receive(), if delivery.is_none() => {
                let Some(message) = message else { break; };
                delivery = Some(message);
            }
        }
    }
    binding.stop_worker();
    inner.cancel_token().cancel();
    if pending.as_ref().is_some_and(|sent| sent.initialize) {
        // 真实应答已进入发送阶段时等 SDK ACK，不插入第二份应答；同时排空背压队列。
        let ((responder, result), ()) = tokio::join!(completed(&mut pending), drain(&mut inner));
        binding.initialize_answered();
        let _ = responder.send(result);
    } else if let Some(failure) = binding.initialize_failure() {
        // rmcp 3.2.0 的初始化应答等待不检查取消；仅 closing 且未应答时唤醒它。
        let wake = inner.send(failure);
        let _ = tokio::join!(wake, drain(&mut inner));
    } else {
        drain(&mut inner).await;
    }
    let _ = inner.close().await;
    // 代理通道也先销毁，迟到 handler 的发送不会继续排队。
    drop(pending);
    drop(delivery);
    drop(requests);
    drop(received);
    binding.worker_finished();
}

async fn drain(inner: &mut Inner) {
    // SDK 普通转发也可能正等满队列；关闭后只丢弃，直到 worker 放下最后一个发送端。
    while inner.receive().await.is_some() {}
}

impl Transport<RoleServer> for Worker {
    type Error = Error;

    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let sending = self.sending.clone();
        let binding = self.binding.clone();
        async move {
            if binding.is_stopping() {
                return Err(Error::TransportClosed);
            }
            let (result, received) = oneshot::channel();
            sending
                .send(Sending { message, result })
                .await
                .map_err(|_| Error::TransportClosed)?;
            received.await.map_err(|_| Error::TransportClosed)?
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        let message = tokio::select! {
            biased;
            _ = self.binding.wait_stop() => return None,
            message = self.receiving.recv() => message,
        };
        if self.binding.is_stopping() {
            None
        } else {
            message
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.binding.stop_worker();
        self.binding.wait_worker().await;
        self.receiving.close();
        while self.receiving.try_recv().is_ok() {}
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.binding.stop_worker();
    }
}
