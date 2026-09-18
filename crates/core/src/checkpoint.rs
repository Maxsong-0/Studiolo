//! 跨 owner 一致 checkpoint 的共享类型与 Interface。
//!
//! 本模块只有纯类型与纯判定，不做任何 IO，也不持有任何 owner 的数据库句柄。
//! 协调器经 [`CheckpointParticipant`] 与每个存储 owner 对话：owner 自己导出
//! 已验证的 checkpoint，协调器永远拿不到 `Connection`。

use crate::VaultUuid;
use serde::{Deserialize, Serialize};

/// 参与协调窗口的存储 owner；数组顺序即屏障的固定进入顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointOwner {
    /// canonical Vault 的唯一写者。
    Broker,
    /// 运行平面 Control 数据库。
    Control,
    /// 人侧身份、会话与恢复码。
    Auth,
    /// 权威追加审计。
    Audit,
    /// 双向 Sync 的 BridgeState。
    Bridge,
    /// M3 类型 token 摘要、撤权与实例登记。
    AgentStore,
}

impl CheckpointOwner {
    /// 屏障必须按此固定顺序逐个进入，任何实现都不得自行重排。
    ///
    /// Audit 排在最后：它要保持可用直到其余 owner 的在途操作实际结束
    /// （`Auth→Audit→Auth` 这类完整语义操作必须能走完），才最后进入存储冻结。
    pub const FREEZE_ORDER: [Self; 6] = [
        Self::Broker,
        Self::Control,
        Self::Auth,
        Self::Bridge,
        Self::AgentStore,
        Self::Audit,
    ];

    /// 返回清单里使用的稳定名字。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Broker => "broker",
            Self::Control => "control",
            Self::Auth => "auth",
            Self::Audit => "audit",
            Self::Bridge => "bridge",
            Self::AgentStore => "agent_store",
        }
    }
}

/// owner 在本次窗口中的装配情况；未装配必须显式记录，不得静默跳过。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerAssembly {
    /// 已实际装配，必须参与协调窗口。
    Present,
    /// 明确未装配；清单逐项标记 absent，不代表被忽略。
    Absent,
}

/// checkpoint 的稳定标识；空串与控制字符一律拒绝。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CheckpointId(String);

impl CheckpointId {
    /// 解析一个非空、无控制字符的标识。
    pub fn parse(value: impl Into<String>) -> Result<Self, CheckpointError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) || value.len() > 128 {
            return Err(CheckpointError::InvalidCheckpointId);
        }
        Ok(Self(value))
    }

    /// 返回标识文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CheckpointId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// 冻结的有界等待预算；无界等待不可表达。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FreezeBudget(u32);

impl FreezeBudget {
    /// 单个 owner 允许的最长等待毫秒数上限。
    pub const MAX_MILLIS: u32 = 120_000;

    /// 构造一个不超过上限的等待预算。
    #[must_use]
    pub fn new(millis: u32) -> Option<Self> {
        (millis <= Self::MAX_MILLIS).then_some(Self(millis))
    }

    /// 返回毫秒预算。
    #[must_use]
    pub fn millis(self) -> u32 {
        self.0
    }
}

/// 一次冻结请求；同一 checkpoint 的全部 owner 共用同一请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeRequest {
    /// 本次 checkpoint 的稳定标识。
    pub checkpoint_id: CheckpointId,
    /// 每个 owner 的有界等待预算。
    pub budget: FreezeBudget,
}

/// 排空证明。Prepared 可读，但可读不等于已排空。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrainProof {
    /// 仍在途的语义操作数；必须为 0。
    pub in_flight_operations: u64,
    /// 是否存在 Prepared 记录；只作事实记录，不放宽排空判定。
    pub prepared_readable: bool,
    /// 后台补记队列是否真的已空；成功一次不等于全部 pending 已交付。
    pub background_delivery_pending: u64,
    /// 本地维护写者是否也已被挡住；只暂停 HTTP 不算。
    pub local_maintenance_writers_blocked: bool,
    /// Auth 的会话检查/列表读取等隐式写入是否也已挡住。
    pub implicit_read_writes_blocked: bool,
}

impl DrainProof {
    /// 是否构成真正的排空；任一条不满足都不算冻结成功。
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.in_flight_operations == 0
            && self.background_delivery_pending == 0
            && self.local_maintenance_writers_blocked
            && self.implicit_read_writes_blocked
    }
}

/// owner 对一次冻结请求的回执。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeReceipt {
    /// 回执来源 owner；协调器按固定顺序逐项核对。
    pub owner: CheckpointOwner,
    /// 该 owner 自己给出的排空证明。
    pub drain: DrainProof,
}

/// SQLite WAL checkpoint 的 busy/result 证据。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalCheckpointProof {
    /// `busy` 返回值；非 0 表示未完成，不能当作已 checkpoint。
    pub busy: i32,
    /// WAL 中的总帧数。
    pub log_frames: i64,
    /// 已回写主库的帧数。
    pub checkpointed_frames: i64,
}

impl WalCheckpointProof {
    /// 是否为完成且非 busy 的 checkpoint。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.busy == 0 && self.log_frames >= 0 && self.checkpointed_frames == self.log_frames
    }
}

/// 各 owner 的水位。语义各不相同，任何一项都不能互相替代。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "owner", rename_all = "snake_case")]
pub enum OwnerWatermark {
    /// Broker：Vault UUID、逻辑 pair_id、提交水位与无未决证明。
    Broker {
        /// 已验证的 Vault 身份。
        vault_uuid: VaultUuid,
        /// 逻辑配对标识；恢复到新目录时必须原样保留。
        pair_id: VaultUuid,
        /// 已完成并持久发布的最大提交序号。
        committed_through: u64,
        /// 是否仍有未决操作；true 不能封存。
        unresolved: bool,
    },
    /// Control：schema、Vault UUID 与 outbox 状态。
    Control {
        /// 运行平面 schema 版本。
        schema_version: u32,
        /// 绑定的 Vault 身份。
        vault_uuid: VaultUuid,
        /// 仍未投递的 outbox 条目数。
        outbox_pending: u64,
        /// 已投递到的 outbox 水位。
        outbox_delivered_through: u64,
    },
    /// Auth：schema、主人绑定与审计锚点。
    Auth {
        /// Auth schema 版本。
        schema_version: u32,
        /// 主人绑定的脱敏摘要（64 位十六进制），不含任何秘密。
        owner_binding_digest: String,
        /// Auth 写入所锚定的审计 offset。
        audit_anchor_offset: u64,
    },
    /// Audit：最后 offset 与完整性链哈希。
    Audit {
        /// 截止点的最后 offset。
        last_offset: u64,
        /// 截止点的记录哈希（64 位十六进制）。
        last_hash: String,
    },
    /// Bridge：link/scope/共同基线/观察序号与持久门禁。
    Bridge {
        /// 固定 link 标识。
        link_id: String,
        /// 已批准同步范围的摘要（64 位十六进制）。
        scope_digest: String,
        /// 共同基线摘要（64 位十六进制）；不能用索引 generation 替代。
        common_base_digest: String,
        /// 本地观察序号；不能用最后一次 Sync 退出时间替代。
        observation_sequence: u64,
        /// 持久门禁是否已关闭。
        durable_gate_closed: bool,
    },
    /// AgentStore：schema、类型 token 摘要与实例登记。
    AgentStore {
        /// AgentStore schema 版本。
        schema_version: u32,
        /// 类型 token 摘要（64 位十六进制），不含 token 本体。
        type_token_digest: String,
        /// 已登记实例数。
        instance_count: u64,
    },
}

impl OwnerWatermark {
    /// 返回该水位所属的 owner。
    #[must_use]
    pub fn owner(&self) -> CheckpointOwner {
        match self {
            Self::Broker { .. } => CheckpointOwner::Broker,
            Self::Control { .. } => CheckpointOwner::Control,
            Self::Auth { .. } => CheckpointOwner::Auth,
            Self::Audit { .. } => CheckpointOwner::Audit,
            Self::Bridge { .. } => CheckpointOwner::Bridge,
            Self::AgentStore { .. } => CheckpointOwner::AgentStore,
        }
    }
}

/// owner 自己导出、且已重开核验过的 checkpoint 票据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedOwnerCheckpoint {
    /// 导出该副本的 owner。
    pub owner: CheckpointOwner,
    /// 固定 schema 版本；未知版本失败关闭。
    pub schema_version: u32,
    /// WAL checkpoint 的 busy/result 证据。
    pub wal: WalCheckpointProof,
    /// 目标文件已 fsync。
    pub target_synced: bool,
    /// 目标文件的父目录已 fsync。
    pub parent_directory_synced: bool,
    /// 副本重开后已核验 schema 与身份。
    pub reopen_verified: bool,
    /// 复制前后固定源绑定未变化。
    pub source_binding_stable: bool,
    /// 该 owner 的水位。
    pub watermark: OwnerWatermark,
}

impl SealedOwnerCheckpoint {
    /// 是否是一份可用于封存票据的已验证 checkpoint。
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.owner == self.watermark.owner()
            && self.wal.is_complete()
            && self.target_synced
            && self.parent_directory_synced
            && self.reopen_verified
            && self.source_binding_stable
    }
}

/// 跨 owner 屏障与导出的 Interface；owner 自己实现，协调器不接触连接。
pub trait CheckpointParticipant: Send + Sync {
    /// 该参与者代表哪个 owner。
    fn owner(&self) -> CheckpointOwner;

    /// 该 owner 当前是否已装配。
    fn assembly(&self) -> OwnerAssembly;

    /// 在有界等待内冻结自己的全部变更入口，并返回排空证明。
    fn freeze(&self, request: &FreezeRequest) -> Result<FreezeReceipt, CheckpointError>;

    /// 导出自己经过验证的 checkpoint 副本。
    fn export(&self, request: &FreezeRequest) -> Result<SealedOwnerCheckpoint, CheckpointError>;

    /// 解冻；协调器在成功与失败路径上都会调用。
    fn thaw(&self) -> Result<(), CheckpointError>;
}

/// 跨 owner 协调窗口的失败模式。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// checkpoint 标识为空、过长或含控制字符。
    #[error("checkpoint 标识不合法")]
    InvalidCheckpointId,
    /// 某个已装配 owner 没有参与本次窗口。
    #[error("owner {owner:?} 未参与协调窗口")]
    OwnerMissing {
        /// 缺席的 owner。
        owner: CheckpointOwner,
    },
    /// 参与者顺序与固定顺序不符。
    #[error("owner {owner:?} 不在固定屏障顺序上")]
    OwnerOutOfOrder {
        /// 顺序错误处的 owner。
        owner: CheckpointOwner,
    },
    /// 同一 owner 被登记了多次。
    #[error("owner {owner:?} 重复登记")]
    OwnerDuplicated {
        /// 重复登记的 owner。
        owner: CheckpointOwner,
    },
    /// 在有界等待内没有排空。
    #[error("owner {owner:?} 在预算内未排空")]
    NotDrained {
        /// 未排空的 owner。
        owner: CheckpointOwner,
    },
    /// 冻结等待超过预算。
    #[error("owner {owner:?} 冻结等待超时")]
    FreezeTimeout {
        /// 超时的 owner。
        owner: CheckpointOwner,
    },
    /// 导出的副本没有通过验证。
    #[error("owner {owner:?} 导出的 checkpoint 未通过验证")]
    Unverified {
        /// 未通过验证的 owner。
        owner: CheckpointOwner,
    },
    /// 该 owner 当前不可用。
    #[error("owner {owner:?} 不可用")]
    OwnerUnavailable {
        /// 不可用的 owner。
        owner: CheckpointOwner,
    },
}

/// 毫秒时间戳；只用于计算恢复点年龄，不引入时区语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstantMillis(i64);

impl InstantMillis {
    /// 从 Unix 毫秒构造。
    #[must_use]
    pub fn from_millis(value: i64) -> Self {
        Self(value)
    }

    /// 返回 Unix 毫秒。
    #[must_use]
    pub fn millis(self) -> i64 {
        self.0
    }
}

/// 恢复点年龄。时钟回退或时间证据缺失一律显示未知。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RecoveryPointAge {
    /// 已知年龄，从内容截止时刻起算。
    Known {
        /// 年龄毫秒数。
        millis: u64,
    },
    /// 未知年龄；不得显示为 0 或绿色。
    Unknown {
        /// 未知的具体原因。
        reason: AgeUnknownReason,
    },
}

/// 年龄未知的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeUnknownReason {
    /// 当前时刻早于内容截止时刻，本地时钟发生回退。
    ClockWentBackwards,
    /// 缺少内容截止时刻等时间证据。
    MissingTimeEvidence,
}

/// 按内容截止时刻计算恢复点年龄；`now` 早于截止时刻即判为时钟回退。
#[must_use]
pub fn recovery_point_age(
    content_cutoff: Option<InstantMillis>,
    now: Option<InstantMillis>,
) -> RecoveryPointAge {
    let (Some(cutoff), Some(now)) = (content_cutoff, now) else {
        return RecoveryPointAge::Unknown {
            reason: AgeUnknownReason::MissingTimeEvidence,
        };
    };
    match now.millis().checked_sub(cutoff.millis()) {
        Some(delta) if delta >= 0 => RecoveryPointAge::Known {
            millis: delta.unsigned_abs(),
        },
        _ => RecoveryPointAge::Unknown {
            reason: AgeUnknownReason::ClockWentBackwards,
        },
    }
}
