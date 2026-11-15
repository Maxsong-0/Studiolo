//! 备份只读投影与可信生产者的单作业审计端口。
//!
//! 本 Module **不新增任何对外入口**：它只把 `studiolo-backup` 的判定结果翻译成
//! 界面要显示的句子，由既有健康页（`/api/v1/health`，走既有鉴权门禁）承载。
//! 显示投影不读磁盘、不连远端、不碰配置；审计端口只使用启动保留的原账本。

pub mod admission;
pub mod audit;
mod evidence;
mod model;
pub mod producer;

pub use evidence::{BackupEvidence, BackupSnapshot};
pub use model::{BackupSection, Severity};
