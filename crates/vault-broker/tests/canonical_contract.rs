//! T-022：`canonical` 读句柄没有创建或修改的权限。
#![cfg(unix)]

#[path = "canonical/identity.rs"]
mod identity;
#[cfg(target_os = "linux")]
#[path = "canonical/linux.rs"]
mod linux;
#[path = "canonical/reads.rs"]
mod reads;
#[path = "canonical/support.rs"]
mod support;
