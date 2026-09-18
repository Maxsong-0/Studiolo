//! 备份与恢复的主人审批：默认关闭，只有固定包、数字预算与「去哪拿」。
//!
//! **备份仓库口令、NAS 凭据与恢复身份材料不在这里，也不允许出现在这里。**
//! 真实秘密只经环境变量注入；仓库、Vault 与本文档只写变量名，不写值。
//!
//! 本 Module 不打开文件、不读环境、不连接任何外部目标，也不判断目标是否真的可达：
//! 「声明了不可变保护」永远不等于「具备不可变保护」，后者由运行侧另行证明。

mod document;
mod types;

pub use document::{BackupApproval, decode};
pub use types::{
    BackupTargetShape, CredentialReference, ResticPin, RetentionPolicy, ScheduleBudget,
};
