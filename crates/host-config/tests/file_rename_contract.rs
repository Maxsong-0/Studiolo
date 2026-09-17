//! 真实私有配置契约；仅本次 /tmp 夹具，无 Vault 读取、数据库或写者初始化。
#[path = "file_rename/budgets.rs"]
mod budgets;
#[path = "file_rename/config.rs"]
mod config;
#[path = "file_rename/drift.rs"]
mod drift;
#[path = "file_rename/exclusions.rs"]
mod exclusions;
#[path = "file_rename/fixture.rs"]
mod fixture;
#[path = "file_rename/pairs.rs"]
mod pairs;
