//! 双向整文件范围只读契约；仅创建本次 /tmp 合成配置，不运行传输或打开数据库。

#[path = "sync_scope/attachments.rs"]
mod attachments;
#[path = "sync_scope/budgets.rs"]
mod budgets;
#[path = "sync_scope/config.rs"]
mod config;
#[path = "sync_scope/config_paths.rs"]
mod config_paths;
#[path = "sync_scope/drift.rs"]
mod drift;
#[path = "sync_scope/exclusions.rs"]
mod exclusions;
#[path = "sync_scope/fixture.rs"]
mod fixture;
#[path = "sync_scope/observation.rs"]
mod observation;
#[path = "sync_scope/union.rs"]
mod union;
