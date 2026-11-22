//! 逐 owner 的固定目标布局与原水位汇总；本层不创建目录、不签发最终 SealTicket。
use std::path::{Path, PathBuf};
use studiolo_backup::checkpoint::{manifest::AuditCoverageProof, types::OwnerRoster};
use studiolo_core::checkpoint::{OwnerWatermark, SealedOwnerCheckpoint};

/// 新根恢复要放回去的组件在封存里的目录名；导出布局和恢复共用这一份。
pub(super) const CANONICAL: &str = "canonical";
pub(super) const WRITER_DATA: &str = "writer-data";
pub(super) const AUTH: &str = "auth";
pub(super) const AUDIT: &str = "audit";
pub(super) const CONTROL: &str = "control";
pub(super) const AGENTS: &str = "agent";
pub(super) const AGENT_AUDIT: &str = "agent-audit";
pub(super) const POLICY: &str = "policy";

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
            vault: root.join(CANONICAL),
            writer: root.join(WRITER_DATA),
            control: root.join(CONTROL),
            auth: root.join(AUTH),
            bridge: root.join("bridge"),
            transport: root.join("transport"),
            plane: root.join("plane"),
            agents: root.join(AGENTS),
            policy: root.join(POLICY),
            audit: root.join(AUDIT),
            agent_audit: root.join(AGENT_AUDIT),
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
/// 生产用的来源目录，交给 [`ReleasedOwners::seal`](super::ReleasedOwners::seal) 的 `catalog` 参数。
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
