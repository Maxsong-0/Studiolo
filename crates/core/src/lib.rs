//! Studiolo 各模块接缝处共用的值类型与端口。

// 各值类型按职责拆在私有子模块里，经下方 `pub use` 重导出，
// 对外路径仍是 `studiolo_core::类型名`；公开子模块只有 `checkpoint`。
pub mod checkpoint;
mod data_labels;
mod error;
mod fence;
mod ids;
mod permission;
mod revision;
mod revision_hasher;
mod vault_path;

pub use data_labels::{BackupScope, Classification, DataLabels, DesktopSync, ModelEgress};
pub use error::{DataLabelError, IdParseError, PathSyntaxError};
pub use fence::Fence;
pub use ids::{AttemptId, TaskId, VaultUuid};
pub use permission::{PermissionSet, ToolId};
pub use revision::Revision;
pub use revision_hasher::RevisionHasher;
pub use vault_path::VaultPath;
