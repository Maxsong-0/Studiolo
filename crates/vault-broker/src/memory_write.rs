//! 结构化记忆先查重；命中仅返回指针，新增必须交回既有批量写入准入。
mod fold;
pub(crate) mod guard;
mod plan;
mod scan;
#[cfg(test)]
mod tests;
use crate::{BatchPreview, BatchRequest, BrokerError, Revision, VaultPath};
use serde::{Deserialize, Serialize};

/// 一事一档的结构化记忆候选；调用方须确认 scope 的真实授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryRequest {
    /// 对应最终批量提交的稳定 ID。
    pub batch_id: String,
    /// 只能位于某个精确的 `03-记忆` 目录或其子目录。
    pub scope: VaultPath,
    /// 一条记忆的类型。
    pub kind: String,
    /// 一条记忆的摘要；与类型共同构成去重键。
    pub summary: String,
    /// 绝对公历日期，严格为 YYYY-MM-DD。
    pub date: String,
    /// 候选正文；命中既有条目时不会覆盖它。
    pub body: String,
}
/// 查重结果或尚未执行的批量创建计划；正文不能持久化到 Control DB。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryPreview {
    /// 已有条目只能另行审阅并提交精确 patch。
    Existing {
        /// 已有条目的 canonical 指针。
        path: VaultPath,
        /// 查重时核验的精确版本。
        revision: Revision,
        /// 查重时已有正文的 SHA-256。
        sha256: String,
        /// 固定为 true，禁止把候选 body 用作全文覆盖。
        requires_patch: bool,
    },
    /// 尚未发生写入；由调用方统一执行 claim/fence/审计与 batch commit。
    Create {
        /// 单条确定路径的 create-only 请求。
        request: BatchRequest,
        /// 精确绑定正文的预览。
        preview: BatchPreview,
    },
}
impl MemoryPreview {
    /// 超过推荐 200 行时供 UI 提示；超过 300 行的候选根本不会返回预览。
    #[must_use]
    pub fn exceeds_target_lines(&self) -> bool {
        match self {
            Self::Create { preview, .. } => preview
                .diffs
                .iter()
                .any(|diff| diff.after.lines().count() > 200),
            Self::Existing { .. } => false,
        }
    }
}
/// Broker 可信枚举与去重能力，不自动写入或授予正文替换权限。
pub trait MemoryWriter: Send + Sync {
    /// 按类型和摘要查重；只有不存在时才返回可审阅的 create-only 批量计划。
    fn preview_memory(&self, request: &MemoryEntryRequest) -> Result<MemoryPreview, BrokerError>;
}
