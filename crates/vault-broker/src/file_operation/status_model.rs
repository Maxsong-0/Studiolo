//! 执行事实和审计闭合分开，prepare 成功不等于可以删除。
use super::{
    audit_model::FileOperationKind,
    model::{FileOperationOutcome, FileOperationReceipt, FilePresence},
};
use crate::VaultPath;
use serde::{Deserialize, Serialize};

/// 同一主日志内的类型化文件阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationPhase {
    /// 已持久目标安装意图，尚无目标成功证明。
    TargetInstallIssued,
    /// 目标创建已持久确认；重开不能自动继续删源。
    TargetInstalled,
    /// 目标已确认，源摘名已发出但效果尚未知。
    SourceDetachIssued,
    /// 两个路径的执行效果与目录同步均已持久确认。
    SourceDetached,
    /// 已占主槽并冻结请求，尚未完成所有 L0 保全。
    Preparing,
    /// 保全已完成，或已固定无可见效果的 CAS 拒绝；等待真实 Intent ACK。
    Prepared,
    /// 已持久发出摘名意图，尚无确切后继效果。
    DetachIssued,
    /// 摘名、完整字节核验和父目录同步都有持久证据。
    Detached,
    /// 已持久发出恢复安装意图，尚无确切后继效果。
    InstallIssued,
    /// 恢复创建及完整核验和目录同步都有持久证据。
    Installed,
    /// 原执行终态已固定，Outcome 未闭合前仍占同一主槽。
    FinalizedAwaitAudit,
}

/// 票据给装配的固定预览；已拒绝的请求不能因为现况回退而再次执行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileOperationPreview {
    /// 固定操作选择器。
    pub operation_id: String,
    /// 固定路径。
    pub path: VaultPath,
    /// 原动作及其条件。
    pub kind: FileOperationKind,
    /// prepare 时实际观察到的状态。
    pub observed_before: FilePresence,
    /// 已经固定的结果；新准备阶段只可能是 Rejected；历史票据可带已固定终态。
    pub fixed_outcome: Option<FileOperationOutcome>,
}

/// 只读操作状态；None 终态不能从当前文件内容补成成功。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileOperationStatus {
    /// 固定操作选择器。
    pub operation_id: String,
    /// 原阶段；已有终态的历史请求为 FinalizedAwaitAudit，是否闭合由 audit_pending 表示。
    pub phase: FileOperationPhase,
    /// 任一固定事件尚未真正 ACK。
    pub audit_pending: bool,
    /// 原请求的固定预览。
    pub preview: FileOperationPreview,
    /// 仅由持久终态派生的执行回执。
    pub receipt: Option<FileOperationReceipt>,
}
