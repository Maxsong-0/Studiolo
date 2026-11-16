//! 逐 owner 的固定目标布局与原水位汇总；本层不创建目录、不签发最终 SealTicket。
use super::*;
use std::path::{Path, PathBuf};
use studiolo_backup::checkpoint::types::OwnerRoster;
use studiolo_core::checkpoint::{
    AuditChainCutoff, OwnerAssembly, OwnerWatermark, SealedOwnerCheckpoint,
};

/// 协调器在专用 staging 里预建的固定子目录；后续总封存将覆盖整个布局。
#[derive(Clone)]
pub struct OwnerExportPaths {
    pub(super) vault: PathBuf,
    pub(super) writer: PathBuf,
    pub(super) control: PathBuf,
    pub(super) auth: PathBuf,
    pub(super) bridge: PathBuf,
    pub(super) transport: PathBuf,
    pub(super) plane: PathBuf,
    pub(super) agents: PathBuf,
    pub(super) policy: PathBuf,
    pub(super) audit: PathBuf,
    pub(super) agent_audit: PathBuf,
}
impl OwnerExportPaths {
    /// 纯路径布局；实际身份、空间、私有权限和独立性由 staging owner 与各源 owner 复验。
    #[must_use]
    pub fn under(root: &Path) -> Self {
        Self {
            vault: root.join("canonical"),
            writer: root.join("writer-data"),
            control: root.join("control"),
            auth: root.join("auth"),
            bridge: root.join("bridge"),
            transport: root.join("transport"),
            plane: root.join("plane"),
            agents: root.join("agent"),
            policy: root.join("policy"),
            audit: root.join("audit"),
            agent_audit: root.join("agent-audit"),
        }
    }
    /// 固定目录集；不允许调用方扩展任意目标名。
    pub fn directories(&self) -> [&Path; 11] {
        [
            &self.vault,
            &self.writer,
            &self.control,
            &self.auth,
            &self.bridge,
            &self.transport,
            &self.plane,
            &self.agents,
            &self.policy,
            &self.audit,
            &self.agent_audit,
        ]
    }
}
/// 所有实际 owner 导出成功的中间结果；未完成 inclusion/manifest 原子总封存，不能提交 worker。
pub struct CheckpointExports {
    /// 固定顺序的实际 present/absent 登记。
    pub roster: OwnerRoster,
    /// 所有 present owner 的真实已验证导出票据。
    pub owners: Vec<SealedOwnerCheckpoint>,
    /// 实际逐条核验的总次数，包含本作业原 Intent。
    pub audit_coverage: AuditCoverageProof,
}
impl FrozenOwners {
    /// 保留全部真实冻结屏障直至过滤视图与本地清单原子封存及重开复验结束。
    pub fn seal(
        &self,
        area: studiolo_backup::checkpoint::staging::StagingArea,
        catalog: impl FnOnce(
            &Path,
            &CheckpointExports,
        ) -> std::result::Result<
            studiolo_backup::checkpoint::staging::SourceCatalog,
            studiolo_backup::checkpoint::staging::StagingError,
        >,
        mut manifest: studiolo_backup::checkpoint::manifest::ManifestInputs,
        times: CheckpointTimes,
    ) -> std::result::Result<
        studiolo_backup::checkpoint::staging::SealedCheckpoint,
        studiolo_backup::checkpoint::staging::StagingError,
    > {
        use studiolo_backup::checkpoint::staging::{SealInputs, StagingError};
        let started = Instant::now();
        if self.paths.directories() != OwnerExportPaths::under(area.export_root()).directories()
            || area.deadline() > self.deadline
        {
            return Err(StagingError::UnsafeRoot);
        }
        let exports = self.export().map_err(|_| StagingError::Unavailable)?;
        // 只在真实副本已经形成、原窗口仍冻结时计算当前登记；不得按路径补默认标签。
        let catalog = catalog(area.export_root(), &exports)?;
        let CheckpointExports {
            roster,
            owners,
            audit_coverage,
        } = exports;
        manifest.audit_coverage = audit_coverage;
        let elapsed =
            i64::try_from(started.elapsed().as_millis()).map_err(|_| StagingError::Expired)?;
        let now = times
            .now
            .millis()
            .checked_add(elapsed)
            .ok_or(StagingError::Expired)?;
        let sealed = area.seal(
            &catalog,
            SealInputs {
                roster,
                owners,
                manifest,
                content_cutoff: times.content_cutoff,
                expires_at: times.expires_at,
                now: studiolo_core::checkpoint::InstantMillis::from_millis(now),
            },
        )?;
        check_deadline(self.deadline, Owner::Audit).map_err(|_| StagingError::Expired)?;
        Ok(sealed)
    }
    /// 仍持有全部屏障与原租约，逐项导出并汇总两个独立审计截止点。
    pub fn export(&self) -> Result<CheckpointExports> {
        let coverage = self.coverage.ok_or_else(|| unavailable(Owner::Audit))?;
        check_deadline(self.deadline, Owner::Broker)?;
        let p = &self.paths;
        let mut owners = vec![
            self.broker
                .export(&p.vault, &p.writer)
                .map_err(|_| unavailable(Owner::Broker))?,
        ];
        if let Some(control) = &self.control {
            owners.push(
                control
                    .export(&p.control)
                    .map_err(|_| unavailable(Owner::Control))?,
            );
        }
        owners.push(
            self.auth
                .export(&p.auth)
                .map_err(|_| unavailable(Owner::Auth))?,
        );
        if let Some(bridge) = &self.bridge {
            owners.push(
                bridge
                    .export(&p.bridge, &p.transport, &p.plane)
                    .map_err(|_| unavailable(Owner::Bridge))?,
            );
        }
        if let Some(agents) = &self.agents {
            owners.push(
                agents
                    .export(&p.agents)
                    .map_err(|_| unavailable(Owner::AgentStore))?,
            );
        }
        if let Some(policy) = &self.policy {
            owners.push(policy.export(&self.request)?);
        }
        let mut audit = self
            .audit
            .export(&p.audit)
            .map_err(|_| unavailable(Owner::Audit))?;
        if let Some(secondary) = &self.agent_audit {
            let sealed = secondary
                .export(&p.agent_audit)
                .map_err(|_| unavailable(Owner::Audit))?;
            if !sealed.is_verified() {
                return Err(unavailable(Owner::Audit));
            }
            let OwnerWatermark::Audit {
                last_offset,
                last_hash,
                agent_ledger: None,
            } = sealed.watermark
            else {
                return Err(unavailable(Owner::Audit));
            };
            let OwnerWatermark::Audit { agent_ledger, .. } = &mut audit.watermark else {
                return Err(unavailable(Owner::Audit));
            };
            *agent_ledger = Some(AuditChainCutoff {
                last_offset,
                last_hash,
            });
        }
        owners.push(audit);
        if owners.iter().any(|owner| !owner.is_verified()) {
            return Err(unavailable(Owner::Audit));
        }
        let present = [
            true,
            self.control.is_some(),
            true,
            self.bridge.is_some(),
            self.agents.is_some(),
            self.policy.is_some(),
            true,
        ];
        let roster = OwnerRoster::new(Owner::FREEZE_ORDER.into_iter().zip(present.map(|yes| {
            if yes {
                OwnerAssembly::Present
            } else {
                OwnerAssembly::Absent
            }
        })))
        .map_err(|_| unavailable(Owner::Audit))?;
        check_deadline(self.deadline, Owner::Audit)?;
        Ok(CheckpointExports {
            roster,
            owners,
            audit_coverage: coverage,
        })
    }
}
/// 生产用的来源目录，交给 [`FrozenOwners::seal`] 的 `catalog` 参数。
///
/// Broker 扫描真实导出给出来源事实，备份层只做组装：尝试正文跟清单里的原批准，
/// 历史正文只继承已登记的逻辑路径，固定 owner 目录由 owner 声明类别。缺来源的尝试文件、
/// 旧清单、孤立历史和未知目录一律整批拒绝，不按路径补默认标签。
pub fn trusted_catalog(
    export_root: &Path,
    exports: &CheckpointExports,
) -> std::result::Result<
    studiolo_backup::checkpoint::staging::SourceCatalog,
    studiolo_backup::checkpoint::staging::StagingError,
> {
    use studiolo_backup::checkpoint::staging::{StagingError, trusted_source_catalog};
    let CheckpointExports { owners, .. } = exports;
    let vault = owners
        .iter()
        .find_map(|owner| match owner.watermark {
            OwnerWatermark::Broker { vault_uuid, .. } => Some(vault_uuid),
            _ => None,
        })
        .ok_or(StagingError::Catalog)?;
    let facts = studiolo_vault_broker::read_trusted_export(export_root, vault)
        .map_err(|_| StagingError::Catalog)?;
    trusted_source_catalog(vault, facts)
}
/// 同一次冻结/封存使用的可信墙钟时刻，与冻结 Instant 截止时间职责不同。
pub struct CheckpointTimes {
    /// 冻结开始时的内容截止时刻。
    pub content_cutoff: studiolo_core::checkpoint::InstantMillis,
    /// 最终传输源的最迟销毁时刻。
    pub expires_at: studiolo_core::checkpoint::InstantMillis,
    /// 过滤计划判定保留期时的可信当前时刻。
    pub now: studiolo_core::checkpoint::InstantMillis,
}
