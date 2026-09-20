//! 对外批量契约；既有正文只接受精确片段补丁。
use crate::{BrokerError, MutationReceipt, Revision, VaultPath};
use serde::{Deserialize, Serialize};

/// 一处精确文本修改；匹配必须唯一，不支持模糊补丁。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextPatch {
    /// 原正文内唯一出现的非空文本。
    pub before: String,
    /// 替换后的文本，可以为空。
    pub after: String,
}
/// 新建 Markdown 或对指定 revision 执行精确补丁。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchEdit {
    /// 仅允许不存在的 Markdown。
    Create {
        /// 新文件全文。
        content: String,
    },
    /// 保留片段之外的原正文；不提供完整覆盖入口。
    Patch {
        /// 必須仍为此版本。
        expected: Revision,
        /// 按顺序应用的唯一片段补丁。
        patches: Vec<TextPatch>,
    },
}
/// 单个逻辑文件的批量修改。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchChange {
    /// canonical 逻辑路径。
    pub path: VaultPath,
    /// create 或精确 patch。
    pub edit: BatchEdit,
}
/// 一个有界、全批 CAS 的写回计划。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    /// 调用者生成的稳定批次 ID，也是幂等键。
    pub batch_id: String,
    /// 单批 1 到 32 个文件，总拟写正文不超过 4 MiB。
    pub changes: Vec<BatchChange>,
}
/// 可审阅的逐文件完整差异；正文不得写入控制数据库。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchDiff {
    /// 目标路径。
    pub path: VaultPath,
    /// 创建为 None，修改为原文。
    pub before: Option<String>,
    /// 候选新文。
    pub after: String,
}
/// 绑定 Vault、请求与每一处差异的确认材料。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchPreview {
    /// 稳定批次 ID。
    pub batch_id: String,
    /// 绑定完整规范请求和差异的 SHA-256。
    pub fingerprint: String,
    /// 按逻辑路径排序的完整差异。
    pub diffs: Vec<BatchDiff>,
}
/// 只有整批完成才返回；回滚也是一笔完整事务。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchReceipt {
    /// 原批次 ID。
    pub batch_id: String,
    /// 原确认预览的指纹。
    pub fingerprint: String,
    /// 原始批量修改的逐项 revision 记录。
    pub mutations: Vec<MutationReceipt>,
    /// 为 true 表示整批已恢复原状态，包括删除本批新建文件。
    pub rolled_back: bool,
}
/// 提供预览、整批提交和带 CAS 的整批回滚能力。
pub trait BatchWriter: Send + Sync {
    /// 只读预览；所有路径和 CAS 必须成功才返回。
    fn preview_batch(&self, request: &BatchRequest) -> Result<BatchPreview, BrokerError>;
    /// 重验预览绑定和全批 CAS，在一个 writer 事务中提交。
    fn commit_batch(
        &mut self,
        request: BatchRequest,
        preview: &BatchPreview,
    ) -> Result<BatchReceipt, BrokerError>;
    /// 所有目标仍等于本批写入版本时，恢复整批原状态。
    fn rollback_batch(&mut self, batch_id: &str) -> Result<BatchReceipt, BrokerError>;
}
