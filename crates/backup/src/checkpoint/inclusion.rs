//! 逐对象 inclusion plan：先证明 ephemeral 不在任一源根内，再按来源与范围决定去留。
//!
//! 元数据缺一项就整批拒绝，绝不按文件名猜；只要有排除项就必须走专用过滤 staging。

use super::types::{SealError, TransferSource, TransferSourceKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use studiolo_core::{
    BackupScope, DataLabels, Revision, VaultPath,
    checkpoint::{CheckpointOwner, InstantMillis},
};

/// 逐对象来源分类；由来源登记给出，不由文件名推断。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceCategory {
    /// canonical Vault 内容。
    Canonical,
    /// Control 数据库副本。
    ControlDatabase,
    /// 权威审计副本。
    AuditLedger,
    /// 完整范围内的 L0 历史。
    FullScopeL0,
    /// 部署清单。
    DeploymentManifest,
    /// 备份清单。
    BackupManifest,
    /// 普通来源资料，按标签的备份范围与到期语义处理。
    SourceMaterial,
    /// 临时数据；不得位于任一源根内。
    Ephemeral,
}

impl SourceCategory {
    /// 是否属于不可被任意排除的必含类别。
    #[must_use]
    pub fn is_mandatory(self) -> bool {
        matches!(
            self,
            Self::Canonical
                | Self::ControlDatabase
                | Self::AuditLedger
                | Self::FullScopeL0
                | Self::DeploymentManifest
                | Self::BackupManifest
        )
    }
}

/// 一条来源登记。任一字段缺失都会导致整批拒绝。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRecord {
    /// 逻辑路径。
    pub path: VaultPath,
    /// 来源分类。
    pub category: SourceCategory,
    /// 分类/备份范围/同步与外发标签。
    pub labels: Option<DataLabels>,
    /// 到期时刻；普通来源资料必须有。
    pub expires_at: Option<InstantMillis>,
    /// 是否已被 crypto-erase。
    pub crypto_erased: bool,
    /// 来源登记声明的 revision。
    pub declared_revision: Option<Revision>,
    /// 生成 plan 时实际复核到的 revision。
    pub observed_revision: Option<Revision>,
}

/// 单个对象最终的处理方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// 完整对象进入备份。
    FullObject,
    /// 只进入脱敏摘要与清单。
    RedactedSummary,
    /// 只进入非正文墓碑。
    NonContentTombstone,
}

/// plan 中的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionEntry {
    /// 逻辑路径。
    pub path: VaultPath,
    /// 来源分类。
    pub category: SourceCategory,
    /// 处理方式。
    pub disposition: Disposition,
}

/// 组装 plan 需要的输入。
#[derive(Clone, Debug)]
pub struct InclusionRequest {
    /// 全部来源根；ephemeral 根不得落在其中任何一个之内。
    pub source_roots: Vec<PathBuf>,
    /// ephemeral 根。
    pub ephemeral_root: PathBuf,
    /// 逐对象来源登记。
    pub records: Vec<SourceRecord>,
    /// plan 生成时刻，用于判定到期。
    pub now: InstantMillis,
}

/// 逐对象 inclusion plan。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionPlan {
    /// 逐项处理方式。
    pub entries: Vec<InclusionEntry>,
    /// 是否存在来源排除项；true 时必须走专用过滤 staging。
    pub requires_filter_staging: bool,
}

impl InclusionPlan {
    /// 组装 plan。任一条记录元数据缺失或 revision 不匹配都整批拒绝。
    pub fn build(request: &InclusionRequest) -> Result<Self, InclusionError> {
        for root in &request.source_roots {
            if is_within(&request.ephemeral_root, root) {
                return Err(InclusionError::EphemeralInsideSourceRoot);
            }
        }
        let mut entries = Vec::with_capacity(request.records.len());
        let mut requires_filter_staging = false;
        for record in &request.records {
            if record.category == SourceCategory::Ephemeral {
                return Err(InclusionError::EphemeralInsideSourceRoot);
            }
            let Some(labels) = &record.labels else {
                return Err(InclusionError::BatchRejected {
                    path: record.path.clone(),
                    reason: RejectionReason::MissingClassificationOrScope,
                });
            };
            let (Some(declared), Some(observed)) =
                (record.declared_revision, record.observed_revision)
            else {
                return Err(InclusionError::BatchRejected {
                    path: record.path.clone(),
                    reason: RejectionReason::MissingRevisionEvidence,
                });
            };
            if declared != observed {
                return Err(InclusionError::BatchRejected {
                    path: record.path.clone(),
                    reason: RejectionReason::RevisionMismatch,
                });
            }
            if record.category == SourceCategory::SourceMaterial && record.expires_at.is_none() {
                return Err(InclusionError::BatchRejected {
                    path: record.path.clone(),
                    reason: RejectionReason::MissingRetentionMetadata,
                });
            }
            let disposition = if record.category.is_mandatory() {
                if labels.backup_scope() != BackupScope::Full {
                    return Err(InclusionError::MandatoryExcluded {
                        path: record.path.clone(),
                        category: record.category,
                    });
                }
                Disposition::FullObject
            } else {
                let expired = record
                    .expires_at
                    .is_some_and(|expiry| expiry <= request.now);
                if record.crypto_erased || expired {
                    Disposition::NonContentTombstone
                } else {
                    match labels.backup_scope() {
                        BackupScope::Full => Disposition::FullObject,
                        BackupScope::SummaryOnly => Disposition::RedactedSummary,
                        BackupScope::None => Disposition::NonContentTombstone,
                    }
                }
            };
            if disposition != Disposition::FullObject {
                requires_filter_staging = true;
            }
            entries.push(InclusionEntry {
                path: record.path.clone(),
                category: record.category,
                disposition,
            });
        }
        Ok(Self {
            entries,
            requires_filter_staging,
        })
    }

    /// 把 plan 绑定到传输源；有排除项就必须是位于两个保护根之外的专用过滤 staging。
    pub fn bind_transfer_source(
        &self,
        source: &TransferSource,
        vault_root: &Path,
        workbench_data_root: &Path,
        shortest_source_retention: InstantMillis,
    ) -> Result<(), InclusionError> {
        source
            .validate(vault_root, workbench_data_root, shortest_source_retention)
            .map_err(InclusionError::Seal)?;
        if self.requires_filter_staging && source.kind != TransferSourceKind::FilteredStaging {
            return Err(InclusionError::FilterStagingRequired);
        }
        Ok(())
    }
}

fn is_within(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

/// 整批拒绝的具体原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    /// 缺少来源分类或备份范围。
    MissingClassificationOrScope,
    /// 缺少到期元数据。
    MissingRetentionMetadata,
    /// 缺少 revision 证据。
    MissingRevisionEvidence,
    /// 声明 revision 与复核 revision 不一致。
    RevisionMismatch,
}

/// inclusion plan 的失败模式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InclusionError {
    /// ephemeral 根落在某个来源根之内。
    EphemeralInsideSourceRoot,
    /// 整批拒绝；不按文件名猜任何一项缺失的元数据。
    BatchRejected {
        /// 触发拒绝的路径。
        path: VaultPath,
        /// 拒绝原因。
        reason: RejectionReason,
    },
    /// 必含类别被排除。
    MandatoryExcluded {
        /// 被排除的路径。
        path: VaultPath,
        /// 被排除的类别。
        category: SourceCategory,
    },
    /// 有排除项却没有专用过滤 staging。
    FilterStagingRequired,
    /// 传输源本身不合法。
    Seal(SealError),
}

impl std::fmt::Display for InclusionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EphemeralInsideSourceRoot => formatter.write_str("ephemeral 位于某个来源根之内"),
            Self::BatchRejected { path, reason } => {
                write!(formatter, "整批拒绝：{path} 的元数据不完整（{reason:?}）")
            }
            Self::MandatoryExcluded { path, category } => {
                write!(formatter, "必含类别 {category:?} 的 {path} 被排除")
            }
            Self::FilterStagingRequired => formatter.write_str("存在排除项但没有专用过滤 staging"),
            Self::Seal(error) => write!(formatter, "传输源不合法：{error}"),
        }
    }
}

impl std::error::Error for InclusionError {}

/// 必含类别对应的 owner，用于在清单里回指。
#[must_use]
pub fn mandatory_owner(category: SourceCategory) -> Option<CheckpointOwner> {
    match category {
        SourceCategory::Canonical | SourceCategory::FullScopeL0 => Some(CheckpointOwner::Broker),
        SourceCategory::ControlDatabase => Some(CheckpointOwner::Control),
        SourceCategory::AuditLedger => Some(CheckpointOwner::Audit),
        _ => None,
    }
}
