//! 选择是数据而不是权限；观察证明封闭且不返回正文或文件描述符。
use super::state::{Proofs, Survey};
use crate::{BlobIdentity, Revision, VaultPath, VaultUuid};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// 可信装配转交的相对路径及收窄字节预算；底层不把它称为授权。
#[derive(Clone, Debug)]
pub struct SyncReadRule {
    /// 目录锚点或精确文件。
    pub path: VaultPath,
    /// 1..=256MiB，最终仍受整批上限约束。
    pub max_bytes: u64,
}
/// 同一配置导出的读取数据；不构造根、进程或写能力。
#[derive(Clone, Debug)]
pub struct SyncReadSelection {
    /// 完整存在的目录锚点，至多64个，互不嵌套。
    pub folders: Vec<SyncReadRule>,
    /// 互不重叠的精确文件，至多128个。
    pub files: Vec<SyncReadRule>,
    /// 额外私有/生成区前缀，至多320个；不能覆盖固定硬排除。
    pub excluded_prefixes: Vec<VaultPath>,
}
/// 原始字节事实，不表示 Markdown 策略合规或附件格式有效。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalSyncFile {
    /// 完整原始 SHA-256 和字节数。
    pub identity: BlobIdentity,
    /// 真实 canonical Vault UUID 和路径绑定的 Revision。
    pub revision: Revision,
    /// 全部字节是否构成完整 UTF-8；没有输出正文或探测前缀。
    pub utf8_valid: bool,
}
/// 只有完整观察才能区分 Present 与固定父链 ENOENT。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalSyncPresence {
    /// 存在且全部字节已核验。
    Present(CanonicalSyncFile),
    /// 完整观察与封闭缺失证明共同确认的缺失。
    Missing,
}
/// 同一 Broker 的完整本地批准范围观察；不代表远端下载完整。
/// ```compile_fail
/// let proof: studiolo_vault_broker::CanonicalSyncObservation = serde_json::from_str("{}").unwrap();
/// ```
pub struct CanonicalSyncObservation {
    pub(super) gate: Gate,
    pub(super) entries: BTreeMap<VaultPath, CanonicalSyncPresence>,
    pub(super) proofs: Proofs,
}
impl CanonicalSyncObservation {
    /// 完整排序集合；不能通过此引用修改证明或原始结果。
    pub fn entries(&self) -> &BTreeMap<VaultPath, CanonicalSyncPresence> {
        &self.entries
    }
    /// 本次真实 canonical Vault 身份。
    pub fn vault_uuid(&self) -> VaultUuid {
        self.gate.vault
    }
    /// 观察时确认的提交水位，不代表远端同步水位。
    pub fn committed_through(&self) -> u64 {
        self.gate.committed
    }
}
pub(super) struct Gate {
    pub instance: Arc<()>,
    pub pair: VaultUuid,
    pub vault: VaultUuid,
    pub committed: u64,
}
/// 完整元数据调查；不可反序列化、改范围或迁移到另一个 Broker。
/// ```compile_fail
/// let proof: studiolo_vault_broker::CanonicalSyncSurvey = serde_json::from_str("{}").unwrap();
/// ```
pub struct CanonicalSyncSurvey {
    pub(super) gate: Gate,
    pub(super) survey: Survey,
}
impl CanonicalSyncSurvey {
    /// 仅包含发现文件、精确规则和初始 required；隐藏/排除名字仍封闭。
    pub fn paths(&self) -> &BTreeSet<VaultPath> {
        &self.survey.paths
    }
}
