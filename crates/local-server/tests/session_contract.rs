//! 应用会话 HTTP 公开组合契约；仅临时双 SQLite、虚构签名身份和 RFC 种子。

#[path = "sessions/bodies.rs"]
mod bodies;
#[path = "sessions/bootstrap.rs"]
mod bootstrap;
#[path = "sessions/canonical.rs"]
mod canonical;
#[path = "sessions/concurrency.rs"]
mod concurrency;
#[path = "sessions/durable.rs"]
mod durable;
#[path = "sessions/lifecycle.rs"]
mod lifecycle;
#[path = "sessions/revocation.rs"]
mod revocation;
#[path = "sessions/security.rs"]
mod security;
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;
#[path = "sessions/transport.rs"]
mod transport;
