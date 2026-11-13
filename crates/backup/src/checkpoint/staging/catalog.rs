//! 来源登记必须来自可信 owner/effective policy；本层从不按名字补标签。
use super::{MAX_NODES, Result, StagingError, digest};
use crate::checkpoint::inclusion::{Disposition, InclusionPlan, InclusionRequest, SourceRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use studiolo_core::{Revision, VaultPath, VaultUuid, checkpoint::InstantMillis};

/// 已经由可信脱敏器生成的字节及声明摘要，不能传入尚待过滤的 raw。
#[derive(Clone, Debug)]
pub struct RedactedObject {
    /// 已脱敏摘要，最多 1 MiB。
    pub bytes: Vec<u8>,
    /// 同一个脱敏结果的完整 SHA-256。
    pub sha256: String,
}
/// 一个真实导出对象的来源登记；导出路径和 revision 原逻辑路径分开。
#[derive(Clone, Debug)]
pub struct ObjectRegistration {
    /// 固定 owner 子目录下的导出文件相对路径。
    pub export_path: VaultPath,
    /// revision 所绑定的原 Vault 身份。
    pub vault_uuid: VaultUuid,
    /// 原标签、保留期与声明 revision；observed_revision 由实际读取覆盖。
    pub source: SourceRecord,
    /// summary_only 必须提供；full/none 不接受第二份正文。
    pub redacted: Option<RedactedObject>,
}
/// 有界、无重复的完整登记；未出现在实物中或实物没有登记均失败。
pub struct SourceCatalog(pub(super) BTreeMap<VaultPath, ObjectRegistration>);
impl SourceCatalog {
    /// 只验证登记形状；真正的 revision 与完整性在扫描实际导出时检查。
    pub fn new(records: impl IntoIterator<Item = ObjectRegistration>) -> Result<Self> {
        let mut entries = BTreeMap::new();
        let mut collision = std::collections::BTreeSet::new();
        for record in records {
            let path = &record.export_path;
            let first = path
                .as_str()
                .split('/')
                .next()
                .ok_or(StagingError::Catalog)?;
            if entries.len() as u64 >= MAX_NODES
                || !super::OWNER_DIRECTORIES.contains(&first)
                || path.as_str().split('/').any(|p| p == ".stversions")
                || !collision.insert(path.collision_key())
            {
                return Err(StagingError::Catalog);
            }
            entries.insert(path.clone(), record);
        }
        Ok(Self(entries))
    }
    pub(super) fn inspect(
        &self,
        path: &VaultPath,
        actual: Revision,
        now: InstantMillis,
    ) -> Result<CatalogEntry> {
        let record = self.0.get(path).ok_or(StagingError::Catalog)?;
        let mut source = record.source.clone();
        source.observed_revision = Some(actual);
        let plan = InclusionPlan::build(&InclusionRequest {
            source_roots: Vec::new(),
            ephemeral_root: "/unused-checked-by-staging".into(),
            records: vec![source.clone()],
            now,
        })
        .map_err(|_| StagingError::Catalog)?;
        let disposition = plan.entries[0].disposition;
        let summary = match (disposition, &record.redacted) {
            (Disposition::RedactedSummary, Some(summary)) => {
                if summary.bytes.len() > 1024 * 1024 || summary.sha256 != digest(&summary.bytes) {
                    return Err(StagingError::Catalog);
                }
                Some(summary.sha256.clone())
            }
            (Disposition::RedactedSummary, None) | (_, Some(_)) => {
                return Err(StagingError::Catalog);
            }
            (_, None) => None,
        };
        let summary_path = summary.as_ref().map(|_| summary_path(path)).transpose()?;
        Ok(CatalogEntry {
            export_path: path.clone(),
            vault_uuid: record.vault_uuid,
            source,
            disposition,
            summary_sha256: summary,
            summary_path,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogEntry {
    pub export_path: VaultPath,
    pub vault_uuid: VaultUuid,
    pub source: SourceRecord,
    pub disposition: Disposition,
    pub summary_sha256: Option<String>,
    pub summary_path: Option<VaultPath>,
}
pub(super) fn summary_path(path: &VaultPath) -> Result<VaultPath> {
    VaultPath::parse(format!(
        "summaries/{}.summary",
        digest(path.as_str().as_bytes())
    ))
    .map_err(|_| StagingError::Catalog)
}
