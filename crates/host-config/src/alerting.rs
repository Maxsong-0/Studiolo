//! 健康页与带外告警的主人审批：默认关闭，只有数字预算与组件角色。
//!
//! **收件地址与 SMTP 凭据不在这里，也不允许出现在这里。**
//! 真实发送端只经环境变量 `STUDIOLO_ALERT_SMTP_URL` / `STUDIOLO_ALERT_TO` 注入；
//! 仓库、Vault 与本文档只写"去哪拿"，不写值。
//!
//! 本 Module 不打开文件、不读环境、不连接任何外部服务。

mod document;
mod types;

pub use document::AlertingApproval;
pub use types::{DeadmanSchedule, DiskLimits, DispatchLimits, SyncLimits};
