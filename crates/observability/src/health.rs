//! 健康页：本地只读聚合，覆盖恢复点年龄、各 owner 活性、磁盘与 Sync 配额、
//! 白名单诊断结论。
//!
//! 三条纪律写在类型里：证据缺失一律"未知"而不是"正常"；未建立恢复点固定显示
//! [`NO_RECOVERY_POINT`] 且不显示绿；对外公开摘要结构上装不下路径、主人身份或秘密。

mod aggregate;
mod diagnostics;
mod model;

pub use aggregate::{HealthInputs, NO_RECOVERY_POINT, aggregate};
pub use diagnostics::{
    DiagnosticsError, WhitelistDiagnosis, WhitelistFinding, WhitelistFindingKind,
    WhitelistSnapshot, diagnose,
};
pub use model::{
    AgeEvidence, HealthReport, HealthSection, HealthState, Instant, OwnerLabel, OwnerLabelError,
    OwnerLiveness, PublicSummary, RecoveryPointEvidence, SectionDetailError,
};
