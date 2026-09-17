//! Rename 的公开只读审阅；每项效果描述历史事实，不宣称当前路径仍相同。
use super::MaintenanceCurrent;
use crate::{FileEffect, FileOperationKind, FileOperationPhase, Revision, VaultPath};
use serde::{Deserialize, Serialize};

/// 原重命名目标的第二路径；主 MaintenanceTarget 始终是源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceSecondaryTarget {
    /// 固定目标路径。
    pub path: VaultPath,
    /// 准备时的实际版本；已确认缺失为 None。
    pub previous_revision: Option<Revision>,
    /// 完整目标提案的路径绑定版本；准备未完成时为 None。
    pub target_revision: Option<Revision>,
    /// 审阅时目标的真实可观察性。
    pub current: MaintenanceCurrent,
    /// Prepared 表示准备时已完整形成提案，本次未再观察；执行态表示本次保全观察可用。
    pub proposed_available: bool,
}

/// 维护展示使用的原 Rename 动作与持久子效果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceFileOperation {
    /// 原动作与源 expected、目标路径；本字段只允许 Rename。
    pub kind: FileOperationKind,
    /// 原持久阶段，不能从当前正文推断。
    pub phase: FileOperationPhase,
    /// 恰好两项，固定顺序为目标、源；未发出为 NotApplied。
    pub effects: Vec<FileEffect>,
}
