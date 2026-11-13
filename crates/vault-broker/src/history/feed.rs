//! Broker 私有跨路径索引；16个提交槽最多512行，正文仍只在历史对象里。
pub(super) mod query;
mod source;
pub(super) mod storage;
use crate::{MutationActor, Revision, VaultPath};
use serde::{Deserialize, Serialize};
pub(super) use storage::{record, retire};

/// 一次已完成内容变化的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// 创建了原来不存在的路径。
    Created,
    /// 替换了已有正文。
    Replaced,
    /// 删除了路径。
    Deleted,
    /// 从历史或墓碑恢复。
    Restored,
    /// 改名的源或目标；counterpart指向另一端。
    Renamed,
    /// 完成一个批量写回中的这条路径。
    Batch,
    /// 整批回滚的这条路径。
    BatchRollback,
}
/// 无正文的内容变化投影；来源重新绑定路径历史与原终态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeEntry {
    /// 固定历史条目标识；不能作为文件路径使用。
    pub id: String,
    /// Broker全库提交序号；同一批次可有多条路径。
    pub sequence: u64,
    /// 变化所属逻辑路径。
    pub path: VaultPath,
    /// 固定操作类别。
    pub kind: ChangeKind,
    /// 实际可信发起主体；旧来源没有主体时保持未知。
    pub actor: Option<MutationActor>,
    /// 之前的版本；创建为None。
    pub previous_revision: Option<Revision>,
    /// 之后的版本；删除为None。
    pub revision: Option<Revision>,
    /// 之后正文的字节数；删除为0。
    pub bytes: u64,
    /// 服务器固定的请求时间；旧来源没有时保持未知。
    pub at_ms: Option<i64>,
    /// 改名另一端；其他操作为None。
    pub counterpart: Option<VaultPath>,
}
/// 每次至多读两桶；稀疏历史可返回空页及继续游标。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangePage {
    /// 按提交序号降序、同序号路径升序排列。
    pub entries: Vec<ChangeEntry>,
    /// 延续同一提交上限的游标；None表示已到开头。
    pub next_cursor: Option<String>,
    /// 首次查询的可见提交上限，不随翻页后的新提交增长。
    pub through_sequence: u64,
    /// 本页窗口含缺失桶，不能声称这些早期变化没有发生。
    pub history_incomplete: bool,
    /// 本页窗口遇到按保留策略退役的记录。
    pub retired: bool,
}
