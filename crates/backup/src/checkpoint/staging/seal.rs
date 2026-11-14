//! 清单与视图的摘要一并绑定；最终目录仅接受登记过的四个文件/目录。
mod metadata;
mod read;
use super::catalog::CatalogEntry;
use super::{
    Result, SealedInventory, SourceCatalog, StagingArea, StagingError, check_deadline, digest, tree,
};
use crate::checkpoint::{
    manifest::{CheckpointManifest, ManifestInputs},
    types::{
        OwnerRoster, SealTicket, StagingBudget, StagingMaterialization, TransferSource,
        TransferSourceKind,
    },
};
use metadata::{read, write};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis, SealedOwnerCheckpoint};

/// 真实 owner 导出后的水位及版本化部署事实。
pub struct SealInputs {
    /// 已装配 owner 的完整登记。
    pub roster: OwnerRoster,
    /// 已验证的独立导出票据。
    pub owners: Vec<SealedOwnerCheckpoint>,
    /// 同一窗口逐条核验的审计证据与部署清单。
    pub manifest: ManifestInputs,
    /// 在冻结开始时记录的内容截止时刻。
    pub content_cutoff: InstantMillis,
    /// 专用传输副本的销毁期限。
    pub expires_at: InstantMillis,
    /// 当前可信时钟；不晚于 expires_at。
    pub now: InstantMillis,
}
/// worker 请求所绑定的具体封存，不通过扫描目录猜「最新」尝试。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedReference {
    /// 原 checkpoint 身份。
    pub checkpoint_id: CheckpointId,
    /// seal.json 的真实摘要；不能只信同目录里自报的摘要。
    pub seal_sha256: String,
}
/// 经实际重开复验后的封存；字段不可由调用方填布尔值伪造。
pub struct SealedCheckpoint {
    reference: SealedReference,
    ticket: SealTicket,
    manifest: CheckpointManifest,
    inventory: SealedInventory,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskSeal {
    schema: u32,
    ticket: SealTicket,
    manifest_sha256: String,
    inclusion_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InclusionDocument {
    schema: u32,
    checkpoint_id: CheckpointId,
    entries: Vec<CatalogEntry>,
    view: tree::TreeProof,
}
impl StagingArea {
    /// 必须在 FrozenOwners 仍活着时调用；最终目录与本地 manifest 一次发布后才返还。
    pub fn seal(mut self, catalog: &SourceCatalog, inputs: SealInputs) -> Result<SealedCheckpoint> {
        let result = self.seal_inner(catalog, inputs).and_then(|sealed| {
            self.lifecycle
                .retain(&self.parent, &sealed, self.deadline)?;
            Ok(sealed)
        });
        match result {
            Ok(sealed) => Ok(sealed),
            Err(error) => {
                self.cleanup()?;
                Err(error)
            }
        }
    }
    fn seal_inner(
        &mut self,
        catalog: &SourceCatalog,
        inputs: SealInputs,
    ) -> Result<SealedCheckpoint> {
        self.check()?;
        if inputs.content_cutoff > inputs.now || inputs.now >= inputs.expires_at {
            return Err(StagingError::Expired);
        }
        self.deadline =
            expiry_deadline(self.deadline, Instant::now(), inputs.now, inputs.expires_at)?;
        let materialized = tree::materialize(
            &self.exports,
            Some(&self.data),
            catalog,
            inputs.now,
            self.deadline,
        )?;
        if materialized.catalog.iter().any(|e| {
            e.disposition != crate::checkpoint::inclusion::Disposition::NonContentTombstone
                && e.source
                    .expires_at
                    .is_some_and(|expiry| inputs.expires_at > expiry)
        }) {
            return Err(StagingError::Expired);
        }
        let checked = tree::materialize(&self.exports, None, catalog, inputs.now, self.deadline)?;
        if checked.source != materialized.source
            || checked.catalog != materialized.catalog
            || tree::verify(&self.data, self.deadline)? != materialized.view
        {
            return Err(StagingError::Changed);
        }
        self.check()?;
        let final_root = self.parent_path.join(&self.final_name);
        let ticket = SealTicket {
            checkpoint_id: self.checkpoint_id.clone(),
            roster: inputs.roster,
            owners: inputs.owners,
            transfer_source: TransferSource {
                root: final_root.clone(),
                kind: TransferSourceKind::FilteredStaging,
                materialization: StagingMaterialization::StreamedCopy,
                expires_at: inputs.expires_at,
                budget: StagingBudget::FIXED,
            },
            content_cutoff: inputs.content_cutoff,
        };
        let manifest = CheckpointManifest::build(&ticket, inputs.manifest)
            .map_err(|_| StagingError::Manifest)?;
        let inclusion = InclusionDocument {
            schema: 1,
            checkpoint_id: self.checkpoint_id.clone(),
            entries: materialized.catalog,
            view: materialized.view,
        };
        let manifest_sha256 = write(&self.pending, "manifest.json", &manifest, self.deadline)?;
        let inclusion_sha256 = write(&self.pending, "inclusion.json", &inclusion, self.deadline)?;
        let seal = DiskSeal {
            schema: 1,
            ticket,
            manifest_sha256,
            inclusion_sha256,
        };
        let seal_sha256 = write(&self.pending, "seal.json", &seal, self.deadline)?;
        self.pending.readonly()?;
        self.check()?;
        // 必须先销毁含排除 raw 的原始导出；失败就没有任何可交给 worker 的引用。
        self.exports
            .discard_until(&self.parent, &self.export_name, self.reader, self.deadline)?;
        self.parent.check_path(&self.parent_path)?;
        check_deadline(self.deadline)?;
        self.lifecycle.published = true;
        self.parent.publish(&self.pending_name, &self.final_name)?;
        self.pending.check_path(&final_root)?;
        let reference = SealedReference {
            checkpoint_id: self.checkpoint_id.clone(),
            seal_sha256,
        };
        let mut sealed = SealedCheckpoint::verify_directory(
            &self.pending,
            &final_root,
            &reference,
            inputs.now,
            self.deadline,
        )?;
        if let Some(identity) = self.reader {
            self.pending.share_contents(identity, self.deadline)?;
            // 根仍为 0500：共享子节点全量核验通过后才允许 worker 进入。
            sealed = SealedCheckpoint::verify_directory(
                &self.pending,
                &final_root,
                &reference,
                inputs.now,
                self.deadline,
            )?;
            self.parent.check_path(&self.parent_path)?;
            check_deadline(self.deadline)?;
            self.pending.activate_reader()?;
            self.pending.check_path(&final_root)?;
            self.parent.check_path(&self.parent_path)?;
        }
        check_deadline(self.deadline)?;
        Ok(sealed)
    }
}
impl SealedCheckpoint {
    pub(super) fn retention_evidence(&self) -> Result<String> {
        let hash = |path| {
            self.inventory
                .files()
                .find(|file| file.path().as_str() == path)
                .map(|file| file.sha256().to_owned())
                .ok_or(StagingError::Changed)
        };
        let seal = DiskSeal {
            schema: 1,
            ticket: self.ticket.clone(),
            manifest_sha256: hash("manifest.json")?,
            inclusion_sha256: hash("inclusion.json")?,
        };
        let bytes = serde_json::to_string(&seal).map_err(|_| StagingError::Manifest)?;
        retention_expiry(&self.reference, &bytes)?;
        Ok(bytes)
    }
    /// 已验证的原封存引用。
    #[must_use]
    pub fn reference(&self) -> &SealedReference {
        &self.reference
    }
    /// 已验证的跨 owner 票据。
    #[must_use]
    pub fn ticket(&self) -> &SealTicket {
        &self.ticket
    }
    /// 已验证的本地清单；远端最终提交是独立的后续步骤。
    #[must_use]
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    /// 全部目录及文件的原字节摘要；远端必须完整逐项核对，不能只比较文件数。
    #[must_use]
    pub fn inventory(&self) -> &SealedInventory {
        &self.inventory
    }
    /// 固定名称只取 checkpoint ID 的摘要，不允许 ID 注入路径。
    #[must_use]
    pub fn root_path(parent: &Path, id: &CheckpointId) -> PathBuf {
        parent.join(format!("sealed-{}", digest(id.as_str().as_bytes())))
    }
}
pub(super) fn retention_expiry(reference: &SealedReference, bytes: &str) -> Result<InstantMillis> {
    if bytes.len() > super::io::JOURNAL_LIMIT || digest(bytes.as_bytes()) != reference.seal_sha256 {
        return Err(StagingError::Changed);
    }
    let seal: DiskSeal = serde_json::from_str(bytes).map_err(|_| StagingError::Manifest)?;
    if seal.schema != 1 || seal.ticket.checkpoint_id != reference.checkpoint_id {
        return Err(StagingError::Changed);
    }
    Ok(seal.ticket.transfer_source.expires_at)
}
fn expiry_deadline(
    deadline: Instant,
    started: Instant,
    now: InstantMillis,
    expiry: InstantMillis,
) -> Result<Instant> {
    let remaining = expiry
        .millis()
        .checked_sub(now.millis())
        .filter(|v| *v > 0)
        .ok_or(StagingError::Expired)?;
    let expiry = started
        .checked_add(std::time::Duration::from_millis(remaining.unsigned_abs()))
        .ok_or(StagingError::Expired)?;
    Ok(deadline.min(expiry))
}
