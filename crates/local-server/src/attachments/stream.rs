//! 完整稳定读后按小块发出；完整缓冲与查询许可同生共死。
use axum::body::Bytes;
use futures_core::Stream;
use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::OwnedSemaphorePermit;

pub(super) struct Download {
    content: Vec<u8>,
    offset: usize,
    _permit: OwnedSemaphorePermit,
}

impl Download {
    pub(super) fn new(content: Vec<u8>, permit: OwnedSemaphorePermit) -> Self {
        Self {
            content,
            offset: 0,
            _permit: permit,
        }
    }
}

impl Stream for Download {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.offset == self.content.len() {
            return Poll::Ready(None);
        }
        let end = (self.offset + 16 * 1024).min(self.content.len());
        // 复制小块而非共享整份 Vec：最后一块在传输层排队时不得继续持有 32 MiB。
        let chunk = Bytes::copy_from_slice(&self.content[self.offset..end]);
        self.offset = end;
        Poll::Ready(Some(Ok(chunk)))
    }
}
