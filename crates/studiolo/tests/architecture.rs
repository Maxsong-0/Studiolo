//! 可执行的模块依赖与文件权限边界。

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
#[path = "architecture_cases/ci.rs"]
mod ci;
#[path = "architecture_cases/dependencies.rs"]
mod dependencies;
#[path = "architecture_cases/factor_http.rs"]
mod factor_http;
#[path = "architecture_support/mod.rs"]
mod support;
