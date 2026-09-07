//! 因子HTTP公开契约：真实Unix来源证明、私有双SQLite与虚构签名身份。

#[path = "factors/bodies.rs"]
mod bodies;
#[path = "factors/client.rs"]
mod client;
#[path = "factors/failures.rs"]
mod failures;
#[path = "factors/fixture.rs"]
mod fixture;
#[path = "factors/fixture_requests.rs"]
mod fixture_requests;
#[path = "factors/interleavings.rs"]
mod interleavings;
#[path = "factors/lifecycle.rs"]
mod lifecycle;
#[path = "factors/security.rs"]
mod security;
#[path = "factors/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;
