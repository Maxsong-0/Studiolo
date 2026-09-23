//! 同一 checkpoint ID 的清单：各 owner 水位、固定包摘要、部署清单与脱敏秘密引用。
//!
//! 每个水位各有含义。清单只接受该 owner 自己的水位字段，
//! 索引 generation、最后一次 Sync 退出时间这类「看起来也是个数」的东西一律拒绝。
//! 固定事实（`inputs` 子模块）与失败模式（`error` 子模块）按职责拆出，仍经本模块原路径导出。

mod error;
mod inputs;

pub use error::ManifestError;
pub use inputs::{
    AuditCoverageProof, DeploymentEntry, ManifestInputs, PackageDigest, SecretReference,
    SecretSource,
};

use super::types::{OwnerRoster, SealTicket};
use inputs::check_secret_reference;
use serde::{Deserialize, Serialize};
use studiolo_core::checkpoint::{
    CheckpointId, CheckpointOwner, InstantMillis, OwnerAssembly, OwnerWatermark,
};

/// 清单 schema 版本；未知版本失败关闭。
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

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
        // effective 版本号或发布时间戳都是「一个数」，不能冒充 effective 哈希。
        OwnerWatermark::Policy { effective_hash, .. } => {
            require_digest(effective_hash, CheckpointOwner::Policy, "effective_hash")?;
        }
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
