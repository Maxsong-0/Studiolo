//! 备份状态与最终清单的持久存储。
//!
//! `model` 是纯 DTO 与 Interface；`storage` 只操作固定状态/清单及其临时名，
//! 并持有 `backup.lock` 的原进程独占租约。导出视图由独立 staging 能力负责。

pub mod model;
pub mod storage;
