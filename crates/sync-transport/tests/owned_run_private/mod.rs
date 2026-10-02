//! T-048 B3 契约测试：行为、失败、恢复三类齐备；T-132 P1 加上假 launcher 驱动的真实 adapter 契约。
//!
//! 假后端与假 launcher 都只驱动纯状态机、持久层与线上契约；没有任何用例声称平台受支持——
//! 真实 systemd／cgroup／隔离特权只能在管理员置备后的隔离 Linux 上实测（`platform.rs` 的
//! 忽略用例把「平台契约未验」印进测试输出）。

mod adapters;
mod behaviour;
// 真实 adapter 只能由 Linux 上的探测应答构造（`verify_backend_at` 在别的平台先报 `UnsupportedPlatform`）。
#[cfg(target_os = "linux")]
mod client_lifecycle;
mod exit_order;
mod failure;
#[cfg(target_os = "linux")]
mod launcher_fake;
mod payload_identity;
mod platform;
mod rebind;
mod rebind_store;
mod recovery;
mod schema;
mod store;
mod support;
