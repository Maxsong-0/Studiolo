//! 显式写者模式的真实认证、Broker 捕获与 HTTP 投影；仅使用临时合成内容。

#[path = "writer_http/failures.rs"]
mod failures;
#[path = "writer_http/fixture.rs"]
mod fixture;
#[path = "writer_http/projection.rs"]
mod projection;
#[allow(dead_code)]
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;
