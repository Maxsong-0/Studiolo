//! 备份状态与最终清单的持久存储。
//!
//! `model` 是纯 DTO 与 Interface；`storage` 是本 crate 唯一做真实 IO 的文件，
//! 只创建固定的 `backup-state.json` 与 `manifest.json`（及其固定临时名）。

pub mod model;
pub mod storage;
