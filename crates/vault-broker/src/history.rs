//! L0 内容历史公开投影；保存主体与归档主体分开，未知旧元数据不由 mtime 补推。

pub(crate) mod blob;
mod catalog;
mod catalog_disk;
mod complete;
mod file_operation;
mod rename;
pub(crate) use file_operation::complete as complete_file_operation;
mod evidence;
mod legacy;
mod prune;
mod query;
mod schema;
pub(crate) mod store;
pub(crate) use catalog::preflight;
pub(crate) use complete::complete;

use crate::{MutationActor, Revision, VaultPath, VaultUuid};
use serde::{Deserialize, Serialize};

/// 同一 revision 可以在不同提交中再次出现；id 标识历史条目，不能当路径使用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    /// 该历史条目的不透明标识；同一 revision 的不同条目可有不同 ID。
    pub id: String,
    /// 与 Vault 身份、逻辑路径及该版本原字节绑定的内容 revision。
    pub revision: Revision,
    /// 该版本原始正文的字节数。
    pub bytes: u64,
    /// 已确认提交所绑定的服务器请求时间；不是客户端写草稿的时间。
    pub authored_at_ms: Option<i64>,
    /// 该版本已知的提交主体；旧记录或无认证来源时为 None。
    pub author: Option<MutationActor>,
    /// 该历史条目可靠封存的时间；旧格式无此事实时为 None。
    pub archived_at_ms: Option<i64>,
    /// 可靠封存此条目的主体或摄入来源；旧记录未知时为 None。
    pub archived_by: Option<MutationActor>,
}

/// 有界分页；游标绑定路径与目录版本，历史变化后须从第一页重新读取。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    /// 本页稳定排序的历史条目，不包含尚未公开的提交。
    pub entries: Vec<HistoryEntry>,
    /// 下一页的不透明游标；没有更多条目时为 None。
    pub next_cursor: Option<String>,
}

/// 已重新校验路径、Vault UUID、revision 与内容摘要的历史字节。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryContent {
    /// 读取时重新核对的 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 历史正文所属的逻辑路径。
    pub path: VaultPath,
    /// 本次返回原字节的路径绑定 revision。
    pub revision: Revision,
    /// 已重新核验 revision 与调用方字节预算的完整历史正文。
    pub content: Vec<u8>,
}

/// 保守解释：同时超过版本数量和天数才可淘汰；未知时间不会按龄删除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryRetention {
    /// 至少保留的最近版本数量；整理 API 要求不小于 50。
    pub versions: usize,
    /// 至少保留的已知归档时间跨度，单位为天；整理 API 要求不小于 30。
    pub days: u32,
}

impl Default for HistoryRetention {
    fn default() -> Self {
        Self {
            versions: 50,
            days: 30,
        }
    }
}

/// 一次有界整理；磁盘原字节只有不存在保留引用时才可删除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryPruneReport {
    /// 本次从查询索引退役的条目数；续办既有清理时可以为零。
    pub removed_entries: usize,
    /// 本次实际删除的无保留引用历史对象数。
    pub removed_objects: usize,
    /// 本次整理完成后仍保留在查询索引中的条目数。
    pub retained_entries: usize,
}
