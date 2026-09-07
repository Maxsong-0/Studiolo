//! 认证状态的公开 HTTP 契约；复用真实 Unix 和私有双库夹具，不复制旧测试集合。

#[allow(dead_code)]
#[path = "factors/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "factors/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "factors/fixture_requests.rs"]
mod fixture_requests;
#[allow(dead_code)]
#[path = "factors/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

#[path = "auth_state/common.rs"]
mod common;
#[path = "auth_state/execution.rs"]
mod execution;
#[path = "auth_state/failures.rs"]
mod failures;
#[path = "auth_state/gates.rs"]
mod gates;
#[path = "auth_state/projections.rs"]
mod projections;
#[path = "auth_state/tcp.rs"]
mod tcp;
