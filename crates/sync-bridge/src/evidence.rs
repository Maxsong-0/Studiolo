//! 封闭输入证据；不构造文件系统能力或生产授权。

use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;

/// 装配提供的 SHA-256 或固定根身份摘要。
pub type Digest = [u8; 32];
/// 装配提供的固定逻辑身份。
pub type Identity = [u8; 16];

/// 一条同步链及其当前目录与规则绑定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// 唯一同步链。
    pub link_id: Identity,
    /// Canonical Vault 身份。
    pub canonical_vault: Identity,
    /// 远端 Sync Vault 身份。
    pub remote_vault: Identity,
    /// 经可信装配核对的 canonical 根身份。
    pub canonical_root_token: Digest,
    /// 经可信装配核对的镜像根身份。
    pub mirror_root_token: Digest,
    /// 绑定实际规则内容，不能仅对范围名称计算摘要。
    pub scope_hash: Digest,
    /// 规则变更代次。
    pub scope_epoch: u64,
}

/// 可信 scope adapter 对本次完整联合路径的核准输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedScope {
    /// 当前链路绑定。
    pub binding: Binding,
    /// 精确核准的路径集；不是生产白名单配置。
    pub paths: BTreeSet<VaultPath>,
}

/// 观察或条件写入所在侧。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// Broker 拥有的权威内容。
    Canonical,
    /// Sync 专用镜像。
    Mirror,
}

/// 一份已稳定读取的对象元数据，本纯核不重算正文摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    /// 各侧自己的条件写入版本，不能以另一侧 revision 替代。
    pub revision: Digest,
    /// 实际字节摘要。
    pub sha256: Digest,
    /// 实际字节数；纯核不换算产品套餐单位。
    pub bytes: u64,
}

/// 显式存在或缺失；表中未列出路径不等于缺失。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// 完整观察确认该路径缺失。
    Missing,
    /// 完整观察提供当前版本。
    Present(FileVersion),
}

/// 同一逻辑路径的两个独立版本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    /// Canonical 当前/基线版本。
    pub canonical: Presence,
    /// 镜像当前/基线版本。
    pub mirror: Presence,
}

/// 持久层已经确认的完整共同基线；本纯核不更新它。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baseline {
    /// 原链路绑定。
    pub binding: Binding,
    /// 已持久确认的版本；0 表示尚未建立。
    pub version: u64,
    /// 基线引用的 canonical 观察序号。
    pub canonical_observation: u64,
    /// 基线引用的镜像观察序号。
    pub mirror_observation: u64,
    /// 每项双方同内容或双方 Missing；初次未建立的路径不在此表。
    pub entries: BTreeMap<VaultPath, Pair>,
}

/// 单侧完整观察，不提供分页接口。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    /// 本次扫描的链路绑定。
    pub binding: Binding,
    /// 扫描所在侧。
    pub side: Side,
    /// 严格前进的观察序号。
    pub sequence: u64,
    /// 由可信扫描器提供；本核不能证明底层文件系统观察完整。
    pub complete: bool,
    /// 包含整个 scope.paths；缺失必须显式列出。
    pub entries: BTreeMap<VaultPath, Presence>,
}

/// 持久 owner 提供的规划游标，本纯核不自行推进。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanningCursor {
    /// 游标所属链路。
    pub binding: Binding,
    /// 预期共同基线版本。
    pub baseline_version: u64,
    /// 已分配的最大计划序号。
    pub last_plan_sequence: u64,
    /// 上次纳入计划的 canonical 观察。
    pub last_canonical_observation: u64,
    /// 上次纳入计划的镜像观察。
    pub last_mirror_observation: u64,
}

/// 同时适用于两个方向的全批删除阈值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteLimits {
    /// 超过此删除数量时熔断，等号可过。
    pub max_count: u64,
    /// 万分比阈值；分母固定为原共同基线存活路径数。
    pub max_basis_points: u16,
}

/// 纯规划的元数据与实际对象大小预算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// 不超过 4096 的整批路径上限。
    pub max_paths: usize,
    /// 不超过 4096 的全部输出项上限。
    pub max_outputs: usize,
    /// 装配显式给出的实际字节上限，没有隐含套餐单位。
    pub max_file_bytes: u64,
    /// 已由主线冻结的删除阈值。
    pub deletion: DeleteLimits,
}

impl Limits {
    /// 保守删除默认；附件大小必须显式给出。
    pub fn conservative(max_file_bytes: u64) -> Self {
        Self {
            max_paths: 4096,
            max_outputs: 4096,
            max_file_bytes,
            deletion: DeleteLimits {
                max_count: 5,
                max_basis_points: 1000,
            },
        }
    }
}

/// 可完整比较的冻结提案输入，供持久化和同键重试。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// 本次精确范围核准。
    pub scope: ApprovedScope,
    /// 原完整共同基线。
    pub baseline: Baseline,
    /// Canonical 完整观察。
    pub canonical: Inventory,
    /// 镜像完整观察。
    pub mirror: Inventory,
    /// 持久 owner 提供的序号边界。
    pub cursor: PlanningCursor,
    /// 调用者显式预期的基线版本。
    pub expected_baseline_version: u64,
    /// 必须严格等于游标上次计划序号加一。
    pub plan_sequence: u64,
    /// 该提案冻结的全部预算。
    pub limits: Limits,
}
