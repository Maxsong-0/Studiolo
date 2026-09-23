//! 各存储 owner 在封存截止点的水位。
//!
//! 每个 owner 只报告自己的水位字段；语义各不相同，任何一项都不能互相替代，
//! 也不能用索引 generation、最后一次 Sync 退出时间这类「看起来也是个数」的东西顶替。

use super::CheckpointOwner;
use crate::VaultUuid;
use serde::{Deserialize, Serialize};

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
    /// Policy：M4 策略库的 schema、现行 effective 的版本与哈希、全局吊销状态。
    Policy {
        /// 策略库 schema 版本。
        schema_version: u32,
        /// 现行 effective 的版本水位（全库单调递增的发布版本号）。
        effective_version: u64,
        /// 现行 effective 哈希（64 位小写十六进制）；不能用版本号或发布时间顶替。
        effective_hash: String,
        /// 截止点是否处于全局吊销；恢复后核对吊销时不得把它丢掉。
        global_revoked: bool,
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
            Self::Policy { .. } => CheckpointOwner::Policy,
        }
    }
}
