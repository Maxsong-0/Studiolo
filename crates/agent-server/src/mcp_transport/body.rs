//! SSE 每次交付重验；流持有请求许可和关停登记，断开或到期即释放，不靠响应头返回时计数。
use super::{TransportState, Validity, binding::Binding, manager::Reservation};
use crate::shutdown::InFlight;
use axum::body::{Body, BodyDataStream, Bytes};
use futures_core::Stream;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{sync::OwnedSemaphorePermit, time::Sleep};

pub(super) struct GuardedBody {
    inner: BodyDataStream,
    state: Arc<TransportState>,
    binding: Arc<Binding>,
    _permit: OwnedSemaphorePermit,
    /// POST 在关停排空里的登记：响应体结束或被丢弃才注销，关停据此等它把结果送完（BUG-033）。
    _in_flight: Option<InFlight>,
    reservation: Option<Reservation>,
    timer: Pin<Box<Sleep>>,
    deadline: Option<Instant>,
}

impl GuardedBody {
    pub fn wrap(
        body: Body,
        state: Arc<TransportState>,
        binding: Arc<Binding>,
        permit: OwnedSemaphorePermit,
        in_flight: Option<InFlight>,
        reservation: Option<Reservation>,
        streaming: bool,
    ) -> Body {
        Body::from_stream(Self {
            inner: body.into_data_stream(),
            state,
            binding,
            _permit: permit,
            _in_flight: in_flight,
            reservation,
            timer: Box::pin(tokio::time::sleep(Duration::from_secs(1))),
            deadline: (!streaming).then(|| Instant::now() + Duration::from_secs(30)),
        })
    }
}

impl Stream for GuardedBody {
    type Item = Result<Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let validity = self.state.validity(&self.binding);
        if validity == Validity::Invalid
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.state.manager.discard(self.binding.clone());
            return Poll::Ready(None);
        }
        if self.timer.as_mut().poll(context).is_ready() {
            self.timer
                .as_mut()
                .reset(tokio::time::Instant::now() + Duration::from_secs(1));
            // 重登记定时唤醒，空闲 SSE 也会及时因撤权或到期结束。
            let _ = self.timer.as_mut().poll(context);
        }
        // 身份库读不出来：这次不交付（凭据可能已被吊销），也不结束流、不丢会话；
        // 上面登记的定时唤醒一秒后再判，非流式响应的期限照旧（BUG-030）。
        if validity == Validity::Unavailable {
            return Poll::Pending;
        }
        match Pin::new(&mut self.inner).poll_next(context) {
            Poll::Ready(Some(Ok(bytes))) => {
                if let Some(mut reservation) = self.reservation.take() {
                    reservation.publish();
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            result => result,
        }
    }
}
