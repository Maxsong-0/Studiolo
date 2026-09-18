//! 备份与恢复点的只读投影。
//!
//! 本 Module **不新增任何对外入口**：它只把 `studiolo-backup` 的判定结果翻译成
//! 界面要显示的句子，由既有健康页（`/api/v1/health`，走既有鉴权门禁）承载。
//! 它不读磁盘、不连远端、不碰配置，也不会把「未知」显示成绿色。

mod evidence;
mod model;

pub use evidence::{BackupEvidence, BackupSnapshot};
pub use model::{BackupSection, Severity};
