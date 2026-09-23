//! 主人句柄的值类型：desired 状态与记录、发布结果、版本、effective 摘要与吊销记录。

use serde::{Deserialize, Serialize};

use crate::{Expansion, PolicyKind, Report};

/// 全库待处理接入方提案上限。
pub const MAX_PENDING_PROPOSALS: usize = 256;
/// 每个实例的待处理提案上限。
pub const MAX_PENDING_PER_INSTANCE: usize = 8;

/// desired 的生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredStatus {
    /// 接入方提案，等待主人校验。
    Pending,
    /// 校验通过，可发布。
    Validated,
    /// 校验未通过。
    Rejected,
    /// 已发布为某个版本。
    Activated,
    /// 同 subject 的另一份 desired 已发布，本份作废。
    Superseded,
    /// 主人撤回。
    Withdrawn,
}

impl DesiredStatus {
    /// 库内稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Validated => "validated",
            Self::Rejected => "rejected",
            Self::Activated => "activated",
            Self::Superseded => "superseded",
            Self::Withdrawn => "withdrawn",
        }
    }

    /// 解析稳定名。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "validated" => Some(Self::Validated),
            "rejected" => Some(Self::Rejected),
            "activated" => Some(Self::Activated),
            "superseded" => Some(Self::Superseded),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }
}

/// 提交者：主人或某个接入方实例。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submitter {
    /// 主人（人侧）。
    Owner,
    /// 接入方实例。
    Agent {
        /// 类型名。
        agent_type: String,
        /// 实例 id。
        instance_id: String,
    },
}

/// 一份 desired 输入；`body` 为空表示撤销该 subject。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesiredInput {
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 正文；`None` = 撤销。
    pub body: Option<String>,
}

/// 库中的一份 desired。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredRecord {
    /// id（ULID）。
    pub id: String,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 提交的正文（撤销为空）。
    pub body: Option<String>,
    /// 规范化正文 sha256。
    pub body_sha256: String,
    /// `owner` 或 `agent`。
    pub submitter_kind: String,
    /// 提交方类型（接入方时）。
    pub agent_type: Option<String>,
    /// 提交方实例（接入方时）。
    pub agent_instance: Option<String>,
    /// 提交时间。
    pub submitted_at: String,
    /// 状态。
    pub status: DesiredStatus,
    /// 最近一次校验报告（损坏时为空）。
    pub report: Option<Report>,
    /// 最近一次校验时间。
    pub validated_at: Option<String>,
    /// 发布成的版本号。
    pub activated_version: Option<u64>,
}

/// 一次发布的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    /// 新版本号。
    pub version: u64,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否撤销。
    pub retired: bool,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布后的 effective 哈希。
    pub effective_hash: String,
    /// 发布前的 effective 哈希。
    pub previous_effective_hash: String,
    /// 本次扩大项。
    pub expansions: Vec<Expansion>,
    /// 来源 desired。
    pub desired_id: Option<String>,
    /// 回滚到的历史版本。
    pub rollback_of: Option<u64>,
    /// 本次是否顺带解除了该类型的吊销。
    pub cleared_type_revocation: bool,
    /// 发布者（`owner` / `bootstrap`）。
    pub activated_by: String,
}

/// 版本元信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionMeta {
    /// 版本号。
    pub version: u64,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否撤销。
    pub retired: bool,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布后的 effective 哈希。
    pub effective_hash: String,
    /// 发布前的 effective 哈希。
    pub previous_effective_hash: String,
    /// 发布时间。
    pub activated_at: String,
    /// 发布者。
    pub activated_by: String,
    /// 来源 desired。
    pub desired_id: Option<String>,
    /// 回滚到的历史版本。
    pub rollback_of: Option<u64>,
    /// 发布时记录的扩大项。
    pub expansions: Vec<Expansion>,
}

/// 版本元信息加正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRecord {
    /// 元信息。
    pub meta: VersionMeta,
    /// 正文（撤销版本为空）。
    pub body: Option<String>,
}

/// effective 中的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveEntry {
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 版本。
    pub version: u64,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布时间。
    pub activated_at: String,
}

/// effective 摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveSummary {
    /// 当前 effective 哈希。
    pub effective_hash: String,
    /// 现行各项。
    pub entries: Vec<EffectiveEntry>,
    /// 是否处于全局吊销。
    pub global_revoked: bool,
    /// 待处理 desired 数（pending ＋ validated）。
    pub pending_desired: usize,
}

/// 吊销级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationLevel {
    /// 单个实例（不可恢复）。
    Instance,
    /// 整个类型（重新发布该类型能力集即恢复）。
    Type,
    /// 全局（L3 解除）。
    Global,
}

impl RevocationLevel {
    /// 库内稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Type => "type",
            Self::Global => "global",
        }
    }

    /// 解析稳定名。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "instance" => Some(Self::Instance),
            "type" => Some(Self::Type),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

/// 一条吊销记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationRecord {
    /// 序号。
    pub seq: u64,
    /// 级别。
    pub level: RevocationLevel,
    /// 目标（实例 id／类型名／`*`）。
    pub target: String,
    /// 吊销时间。
    pub revoked_at: String,
    /// 吊销者。
    pub revoked_by: String,
    /// 理由。
    pub reason: String,
    /// 解除时间。
    pub released_at: Option<String>,
    /// 解除者。
    pub released_by: Option<String>,
}

/// 全局吊销的固定目标名。
pub(crate) const GLOBAL_TARGET: &str = "*";
