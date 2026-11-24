//! 可执行的模块依赖与文件权限边界。

#[path = "architecture_cases/audit_permit_forwarding.rs"]
mod audit_permit_forwarding;
#[path = "architecture_cases/audit_storage.rs"]
mod audit_storage;

#[path = "architecture_cases/auth_admin.rs"]
mod auth_admin;
#[path = "architecture_cases/auth_http.rs"]
mod auth_http;
#[path = "architecture_cases/auth_storage.rs"]
mod auth_storage;
#[path = "architecture_cases/authority.rs"]
mod authority;
#[path = "architecture_cases/checkpoint_window.rs"]
mod checkpoint_window;
#[path = "architecture_cases/ci.rs"]
mod ci;
#[path = "architecture_cases/dependencies.rs"]
mod dependencies;
#[path = "architecture_cases/factor_http.rs"]
mod factor_http;
#[path = "architecture_cases/file_lock_release.rs"]
mod file_lock_release;
#[path = "architecture_cases/leftover_sockets.rs"]
mod leftover_sockets;
#[path = "architecture_cases/maintenance.rs"]
mod maintenance;
#[path = "architecture_cases/nextest_groups.rs"]
mod nextest_groups;
#[path = "architecture_cases/policy_storage.rs"]
mod policy_storage;
#[path = "architecture_cases/supply_storage.rs"]
mod supply_storage;
#[path = "architecture_support/mod.rs"]
mod support;
#[path = "architecture_cases/sync_bridge.rs"]
mod sync_bridge;
#[path = "architecture_cases/sync_path_budget.rs"]
mod sync_path_budget;
#[path = "architecture_cases/test_linux_runner.rs"]
mod test_linux_runner;
