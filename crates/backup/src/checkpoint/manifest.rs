//! 同一 checkpoint ID 的清单：各 owner 水位、固定包摘要、部署清单与脱敏秘密引用。
//!
//! 每个水位各有含义。清单只接受该 owner 自己的水位字段，
//! 索引 generation、最后一次 Sync 退出时间这类「看起来也是个数」的东西一律拒绝。

use super::types::{OwnerRoster, SealTicket};
use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::{
    CheckpointId, CheckpointOwner, InstantMillis, OwnerAssembly, OwnerWatermark,
};

/// 清单 schema 版本；未知版本失败关闭。
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// 固定应用或 CLI 包的摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageDigest {
    /// 包名。
    pub name: String,
    /// 固定版本号。
    pub version: String,
    /// 内容摘要（64 位小写十六进制）。
    pub digest: String,
}

/// 部署清单的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentEntry {
    /// 部署单元名。
    pub unit: String,
    /// 该单元的固定 revision。
    pub revision: String,
}

/// 秘密的来源；只保留「去哪拿」，永不保留值本身。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretSource {
    /// 从环境变量注入；这里只记录变量名。
    EnvironmentVariable {
        /// 变量名，必须是大写、下划线与数字。
        variable: String,
    },
}

/// 脱敏后的秘密引用映射。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReference {
    /// 逻辑名，例如 `restic_repository_password`。
    pub logical_name: String,
    /// 去哪拿。
    pub source: SecretSource,
}

/// 已 ACK 事件逐条对应 Audit 截止点内原事件的证明。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditCoverageProof {
    /// 本次逐条核对的已 ACK 事件数。
    pub acked_events_checked: u64,
    /// 其中精确对应到原事件的条数。
    pub acked_events_matched: u64,
    /// 是否只比较了 offset 小于最后 offset；true 表示证明不成立。
    pub offset_comparison_only: bool,
}

impl AuditCoverageProof {
    /// 只有逐条对应且不是纯 offset 比较才算成立。
    #[must_use]
    pub fn is_exact(&self) -> bool {
        !self.offset_comparison_only
            && self.acked_events_checked > 0
            && self.acked_events_checked == self.acked_events_matched
    }
}

/// 清单构建时需要、但不属于任何单个 SQLite owner 的固定事实。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestInputs {
    /// 固定应用包摘要。
    pub application_packages: Vec<PackageDigest>,
    /// 固定 CLI 包摘要。
    pub cli_packages: Vec<PackageDigest>,
    /// 部署清单。
    pub deployment: Vec<DeploymentEntry>,
    /// 脱敏秘密引用映射。
    pub secret_references: Vec<SecretReference>,
    /// 已 ACK 事件逐条对应证明。
    pub audit_coverage: AuditCoverageProof,
}

/// 一次 checkpoint 的完整清单。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointManifest {
    /// 清单 schema 版本。
    pub schema_version: u32,
    /// checkpoint 标识。
    pub checkpoint_id: CheckpointId,
    /// 内容截止时刻。
    pub content_cutoff: InstantMillis,
    /// 逐项标记 present/absent 的 owner 清单。
    pub roster: OwnerRoster,
    /// 各已装配 owner 的水位。
    pub watermarks: Vec<OwnerWatermark>,
    /// 固定应用包摘要。
    pub application_packages: Vec<PackageDigest>,
    /// 固定 CLI 包摘要。
    pub cli_packages: Vec<PackageDigest>,
    /// 部署清单。
    pub deployment: Vec<DeploymentEntry>,
    /// 脱敏秘密引用映射。
    pub secret_references: Vec<SecretReference>,
    /// 已 ACK 事件逐条对应证明。
    pub audit_coverage: AuditCoverageProof,
}

impl CheckpointManifest {
    /// 由封存票据与固定事实组装清单；任一必填水位缺失或被替代即拒绝。
    pub fn build(ticket: &SealTicket, inputs: ManifestInputs) -> Result<Self, ManifestError> {
        let watermarks: Vec<OwnerWatermark> = ticket
            .owners
            .iter()
            .map(|sealed| sealed.watermark.clone())
            .collect();
        for (owner, assembly) in ticket.roster.entries() {
            if assembly == OwnerAssembly::Present
                && !watermarks
                    .iter()
                    .any(|watermark| watermark.owner() == owner)
            {
                return Err(ManifestError::WatermarkMissing { owner });
            }
        }
        for watermark in &watermarks {
            check_watermark(watermark)?;
        }
        if inputs.application_packages.is_empty() || inputs.cli_packages.is_empty() {
            return Err(ManifestError::PackageDigestsMissing);
        }
        for package in inputs
            .application_packages
            .iter()
            .chain(&inputs.cli_packages)
        {
            if package.name.trim().is_empty() || package.version.trim().is_empty() {
                return Err(ManifestError::PackageDigestsMissing);
            }
            require_digest(&package.digest, CheckpointOwner::Broker, "package_digest")?;
        }
        if inputs.deployment.is_empty() {
            return Err(ManifestError::DeploymentManifestMissing);
        }
        for reference in &inputs.secret_references {
            check_secret_reference(reference)?;
        }
        if !inputs.audit_coverage.is_exact() {
            return Err(ManifestError::AuditCoverageNotExact);
        }
        Ok(Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            checkpoint_id: ticket.checkpoint_id.clone(),
            content_cutoff: ticket.content_cutoff,
            roster: ticket.roster.clone(),
            watermarks,
            application_packages: inputs.application_packages,
            cli_packages: inputs.cli_packages,
            deployment: inputs.deployment,
            secret_references: inputs.secret_references,
            audit_coverage: inputs.audit_coverage,
        })
    }

    /// 重新校验一份已持久的清单；未知 schema 版本失败关闭。
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        for (owner, assembly) in self.roster.entries() {
            if assembly == OwnerAssembly::Present
                && !self
                    .watermarks
                    .iter()
                    .any(|watermark| watermark.owner() == owner)
            {
                return Err(ManifestError::WatermarkMissing { owner });
            }
        }
        for watermark in &self.watermarks {
            check_watermark(watermark)?;
        }
        if !self.audit_coverage.is_exact() {
            return Err(ManifestError::AuditCoverageNotExact);
        }
        Ok(())
    }
}

fn check_watermark(watermark: &OwnerWatermark) -> Result<(), ManifestError> {
    match watermark {
        OwnerWatermark::Broker { unresolved, .. } => {
            if *unresolved {
                return Err(ManifestError::BrokerUnresolved);
            }
        }
        OwnerWatermark::Control {
            outbox_pending,
            outbox_delivered_through,
            ..
        } => {
            if *outbox_pending > 0 && *outbox_delivered_through == 0 {
                return Err(ManifestError::ControlOutboxUnsettled);
            }
        }
        OwnerWatermark::Auth {
            owner_binding_digest,
            ..
        } => require_digest(
            owner_binding_digest,
            CheckpointOwner::Auth,
            "owner_binding_digest",
        )?,
        OwnerWatermark::Audit { last_hash, .. } => {
            require_digest(last_hash, CheckpointOwner::Audit, "last_hash")?;
        }
        OwnerWatermark::Bridge {
            link_id,
            scope_digest,
            common_base_digest,
            observation_sequence,
            durable_gate_closed,
        } => {
            if link_id.trim().is_empty() {
                return Err(ManifestError::WatermarkMissing {
                    owner: CheckpointOwner::Bridge,
                });
            }
            require_digest(scope_digest, CheckpointOwner::Bridge, "scope_digest")?;
            require_digest(
                common_base_digest,
                CheckpointOwner::Bridge,
                "common_base_digest",
            )?;
            if *observation_sequence == 0 || !*durable_gate_closed {
                return Err(ManifestError::BridgeGateOpen);
            }
        }
        OwnerWatermark::AgentStore {
            type_token_digest, ..
        } => require_digest(
            type_token_digest,
            CheckpointOwner::AgentStore,
            "type_token_digest",
        )?,
    }
    Ok(())
}

/// 摘要必须是 64 位小写十六进制。纯十进制数字（索引 generation、退出时间戳）
/// 在这里被明确判为「用别的水位顶替」，而不是宽容地放行。
fn require_digest(
    value: &str,
    owner: CheckpointOwner,
    field: &'static str,
) -> Result<(), ManifestError> {
    let is_digest = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if is_digest {
        return Ok(());
    }
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ManifestError::WatermarkSubstituted { owner, field });
    }
    Err(ManifestError::WatermarkMalformed { owner, field })
}

fn check_secret_reference(reference: &SecretReference) -> Result<(), ManifestError> {
    if reference.logical_name.trim().is_empty() || reference.logical_name.len() > 128 {
        return Err(ManifestError::SecretReferenceMalformed);
    }
    let SecretSource::EnvironmentVariable { variable } = &reference.source;
    let shaped = !variable.is_empty()
        && variable.len() <= 128
        && variable
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !variable.starts_with(|c: char| c.is_ascii_digit());
    if !shaped {
        return Err(ManifestError::SecretReferenceMalformed);
    }
    Ok(())
}

/// 清单组装与校验的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// 已装配 owner 缺少自己的水位。
    WatermarkMissing {
        /// 缺失水位的 owner。
        owner: CheckpointOwner,
    },
    /// 水位字段被别的计数顶替（例如索引 generation 或退出时间戳）。
    WatermarkSubstituted {
        /// 被顶替水位的 owner。
        owner: CheckpointOwner,
        /// 被顶替的字段名。
        field: &'static str,
    },
    /// 水位字段格式不合法。
    WatermarkMalformed {
        /// 字段所属 owner。
        owner: CheckpointOwner,
        /// 字段名。
        field: &'static str,
    },
    /// Broker 仍有未决操作。
    BrokerUnresolved,
    /// Control outbox 仍有未投递条目且投递水位为 0。
    ControlOutboxUnsettled,
    /// Bridge 持久门禁未关闭或观察序号为 0。
    BridgeGateOpen,
    /// 缺少固定应用或 CLI 包摘要。
    PackageDigestsMissing,
    /// 缺少部署清单。
    DeploymentManifestMissing,
    /// 秘密引用不是纯粹的「去哪拿」。
    SecretReferenceMalformed,
    /// 已 ACK 事件没有逐条对应原事件。
    AuditCoverageNotExact,
    /// 清单 schema 版本未知。
    UnsupportedSchema {
        /// 读到的版本号。
        found: u32,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WatermarkMissing { owner } => {
                write!(formatter, "owner {} 缺少水位", owner.as_str())
            }
            Self::WatermarkSubstituted { owner, field } => write!(
                formatter,
                "owner {} 的 {field} 被其他计数顶替",
                owner.as_str()
            ),
            Self::WatermarkMalformed { owner, field } => {
                write!(formatter, "owner {} 的 {field} 格式不合法", owner.as_str())
            }
            Self::BrokerUnresolved => formatter.write_str("Broker 仍有未决操作"),
            Self::ControlOutboxUnsettled => formatter.write_str("Control outbox 状态未结清"),
            Self::BridgeGateOpen => formatter.write_str("Bridge 持久门禁未关闭"),
            Self::PackageDigestsMissing => formatter.write_str("缺少固定应用或 CLI 包摘要"),
            Self::DeploymentManifestMissing => formatter.write_str("缺少部署清单"),
            Self::SecretReferenceMalformed => formatter.write_str("秘密引用不是脱敏的来源映射"),
            Self::AuditCoverageNotExact => {
                formatter.write_str("已 ACK 事件没有逐条对应 Audit 截止点内原事件")
            }
            Self::UnsupportedSchema { found } => write!(formatter, "清单 schema 版本 {found} 未知"),
        }
    }
}

impl std::error::Error for ManifestError {}
