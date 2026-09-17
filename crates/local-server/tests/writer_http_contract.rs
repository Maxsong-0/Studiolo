//! 显式写者模式的真实认证、Broker 捕获与 HTTP 投影；仅使用临时合成内容。

#[path = "writer_http/editing.rs"]
mod editing;
#[path = "writer_http/editing_races.rs"]
mod editing_races;
#[path = "writer_http/failures.rs"]
mod failures;
#[path = "writer_http/fixture.rs"]
mod fixture;
#[path = "writer_http/history.rs"]
mod history;
#[path = "writer_http/nonces.rs"]
mod nonces;
#[path = "writer_http/projection.rs"]
mod projection;
#[allow(dead_code)]
#[path = "sessions/support.rs"]
mod support;
#[path = "access/test_support.rs"]
mod test_support;

#[path = "writer_http/file_operation_races.rs"]
mod file_operation_races;
#[path = "writer_http/file_operations.rs"]
mod file_operations;

#[path = "writer_http/file_operation_audit.rs"]
mod file_operation_audit;

#[path = "writer_http/file_operation_pages.rs"]
mod file_operation_pages;

#[path = "writer_http/file_operation_startup.rs"]
mod file_operation_startup;

#[path = "writer_http/file_operation_scope_races.rs"]
mod file_operation_scope_races;

#[path = "writer_http/file_rename.rs"]
mod file_rename;
