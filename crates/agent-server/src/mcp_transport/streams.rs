//! 不依赖 HTTP body 再次被 poll；会话关闭会立即丢弃 SDK 的 SSE 接收端。
use futures_core::Stream;
use rmcp::transport::streamable_http_server::session::ServerSseMessage;
use std::{
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll, Waker},
};

type Inner = Pin<Box<dyn Stream<Item = ServerSseMessage> + Send>>;
struct Source {
    inner: Option<Inner>,
    waker: Option<Waker>,
}
struct Shared(Mutex<Source>);

#[derive(Default)]
struct State {
    closed: bool,
    streams: Vec<Weak<Shared>>,
}

#[derive(Default)]
pub(super) struct Streams(Mutex<State>);

pub(super) struct GuardedStream(Arc<Shared>);

impl Streams {
    pub fn wrap(
        &self,
        stream: impl Stream<Item = ServerSseMessage> + Send + 'static,
    ) -> Option<GuardedStream> {
        let mut state = self.0.lock().ok()?;
        state.streams.retain(|stream| stream.strong_count() > 0);
        // 与全局 HTTP 活流许可同上限；私有 Manager 被单独调用也不会无界登记。
        if state.closed || state.streams.len() >= 16 {
            return None;
        }
        let shared = Arc::new(Shared(Mutex::new(Source {
            inner: Some(Box::pin(stream)),
            waker: None,
        })));
        state.streams.push(Arc::downgrade(&shared));
        Some(GuardedStream(shared))
    }

    pub fn close(&self) {
        let streams = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            std::mem::take(&mut state.streams)
        };
        for stream in streams.into_iter().filter_map(|stream| stream.upgrade()) {
            let (inner, waker) = {
                let mut source = stream.0.lock().unwrap_or_else(|error| error.into_inner());
                (source.inner.take(), source.waker.take())
            };
            drop(inner);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl Stream for GuardedStream {
    type Item = ServerSseMessage;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Ok(mut stream) = self.0.0.lock() else {
            return Poll::Ready(None);
        };
        stream.waker = Some(context.waker().clone());
        match stream.inner.as_mut() {
            Some(stream) => stream.as_mut().poll_next(context),
            None => Poll::Ready(None),
        }
    }
}
