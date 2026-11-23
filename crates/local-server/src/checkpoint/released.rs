//! 冻结窗口放开之后（BUG-045）：各库只剩定格在冻结那一刻的读连接，写者只剩两棵树的文件句柄，
//! 业务照常读写。这里校验登录、控制、接入方库的定格（排空、结构，随表变大），核剩下的回执、逐项导出、
//! 登记来源、封存暂存区；读到的全是冻结那一刻。
//! 没有屏障可以放开：丢弃只关定格的连接和句柄，失败不出票，留下的暂存由暂存层回收。
use super::{
    CheckpointError, CheckpointExports, CheckpointTimes, OwnerExportPaths, PolicyParticipant,
    Result, StepTimes, add, check_deadline, unavailable,
};
use crate::coordinator::broker_failure;
use std::{path::Path, time::Instant};
use studiolo_audit_ledger::FrozenAudit;
use studiolo_auth::FrozenAuth;
use studiolo_backup::checkpoint::{
    manifest::{AuditCoverageProof, ManifestInputs},
    staging::{SealInputs, SealedCheckpoint, SourceCatalog, StagingArea, StagingError},
    types::OwnerRoster,
};
use studiolo_control_db::FrozenControl;
use studiolo_core::checkpoint::{
    AuditChainCutoff, CheckpointOwner as Owner, CheckpointParticipant, FreezeRequest,
    InstantMillis, OwnerAssembly, OwnerWatermark, SealedOwnerCheckpoint,
};
use studiolo_supply::FrozenAgentStore;
use studiolo_vault_broker::BrokerCapture;

/// 由 [`super::FrozenOwners::release`] 交出；字段按冻结的逆序析构。
pub struct ReleasedOwners {
    pub(super) audit: FrozenAudit,
    pub(super) agent_audit: Option<FrozenAudit>,
    pub(super) policy: Option<PolicyParticipant>,
    pub(super) agents: Option<FrozenAgentStore>,
    /// Bridge 在窗口里整份导出过了，这里只留它的票据。
    pub(super) bridge: Option<SealedOwnerCheckpoint>,
    pub(super) auth: FrozenAuth,
    pub(super) control: Option<FrozenControl>,
    pub(super) broker: BrokerCapture,
    pub(super) request: FreezeRequest,
    pub(super) paths: OwnerExportPaths,
    /// 窗口里已经核过的回执数。
    pub(super) checked: u64,
    pub(super) deadline: Instant,
}
/// 放开之后哪一步没做成（BUG-045）：生产者照它写一行日志，账本里照旧只记固定原因。
/// 每一类只带库名和固定类别，不带路径和内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SealFailure {
    /// 校验定格或核回执（登录、控制、接入方库）。
    Verify(CheckpointError),
    /// 逐项导出（含副本上的整本验链和校验）。写者分得出超过固定上限、定格之后被改过（`broker_failure`）。
    Export(CheckpointError),
    /// 登记来源、过滤视图、封存暂存区。
    Staging(StagingError),
}
impl SealFailure {
    /// 记进账本的那一类：校验、导出没做成都按暂存区不可用算（和放开之前一样），暂存层的照它自己的分类。
    pub fn staging(&self) -> StagingError {
        match self {
            Self::Verify(_) | Self::Export(_) => StagingError::Unavailable,
            Self::Staging(error) => *error,
        }
    }
}
impl std::fmt::Display for SealFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verify(error) => write!(f, "校验与核回执：{error}"),
            Self::Export(error) => write!(f, "导出：{error}"),
            Self::Staging(error) => write!(f, "封存暂存区：{error:?}"),
        }
    }
}
impl ReleasedOwners {
    /// 不记分步的封存，只给用例用；生产封存用 [`Self::seal_timed`]。
    #[cfg(test)]
    pub(crate) fn seal(
        &mut self,
        area: StagingArea,
        catalog: impl FnOnce(
            &Path,
            &CheckpointExports,
        ) -> std::result::Result<SourceCatalog, StagingError>,
        manifest: ManifestInputs,
        times: CheckpointTimes,
    ) -> std::result::Result<SealedCheckpoint, SealFailure> {
        self.seal_timed(area, catalog, manifest, times, &mut StepTimes::start())
    }
    /// 逐项导出，过滤视图与本地清单原子封存、重开复验；暂存区的截止时间不能晚于放开时给的截止时间。
    /// 走过的每一步记进 `steps`；失败时已走过的照样留着，生产者照它写日志（BUG-050）。
    pub fn seal_timed(
        &mut self,
        area: StagingArea,
        catalog: impl FnOnce(
            &Path,
            &CheckpointExports,
        ) -> std::result::Result<SourceCatalog, StagingError>,
        mut manifest: ManifestInputs,
        times: CheckpointTimes,
        steps: &mut StepTimes,
    ) -> std::result::Result<SealedCheckpoint, SealFailure> {
        let started = Instant::now();
        if self.paths.directories() != OwnerExportPaths::under(area.export_root()).directories()
            || area.deadline() > self.deadline
        {
            return Err(SealFailure::Staging(StagingError::UnsafeRoot));
        }
        let coverage = self.verify_audit().map_err(SealFailure::Verify)?;
        steps.mark("校验与核回执");
        let exports = self
            .export_owners(coverage, steps)
            .map_err(SealFailure::Export)?;
        // 只在真实副本已经形成之后计算当前登记；不得按路径补默认标签。
        let catalog = catalog(area.export_root(), &exports).map_err(SealFailure::Staging)?;
        steps.mark("登记来源");
        let CheckpointExports {
            roster,
            owners,
            audit_coverage,
        } = exports;
        manifest.audit_coverage = audit_coverage;
        let expired = || SealFailure::Staging(StagingError::Expired);
        let elapsed = i64::try_from(started.elapsed().as_millis()).map_err(|_| expired())?;
        let now = times
            .now
            .millis()
            .checked_add(elapsed)
            .ok_or_else(expired)?;
        let sealed = area
            .seal(
                &catalog,
                SealInputs {
                    roster,
                    owners,
                    manifest,
                    content_cutoff: times.content_cutoff,
                    expires_at: times.expires_at,
                    now: InstantMillis::from_millis(now),
                },
            )
            .map_err(SealFailure::Staging)?;
        check_deadline(self.deadline, Owner::Audit).map_err(|_| expired())?;
        steps.mark("封存暂存区");
        Ok(sealed)
    }
    /// 先校验登录、控制、接入方库的定格并核剩下的回执（都读各自定格的库和定格的账本），
    /// 再按固定顺序逐项导出，汇总两个独立审计截止点。账本的整本验链在它自己的导出里、在副本上做。
    pub fn export(&mut self) -> Result<CheckpointExports> {
        let coverage = self.verify_audit()?;
        self.export_owners(coverage, &mut StepTimes::start())
    }
    /// 按固定顺序逐项导出；回执已经核过。写者、其余各库、两本账本（副本上整本验链）各记一步。
    fn export_owners(
        &mut self,
        coverage: AuditCoverageProof,
        steps: &mut StepTimes,
    ) -> Result<CheckpointExports> {
        let p = &self.paths;
        // 写者分得清超过固定上限、定格之后被改过和别的故障，和窗口里同一套类别（BUG-045）。
        let mut owners = vec![
            self.broker
                .export(&p.vault, &p.writer, self.deadline)
                .map_err(|error| broker_failure(Some(&error)))?,
        ];
        steps.mark("写者导出");
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
            owners.push(bridge.clone());
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
        steps.mark("其余导出");
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
        steps.mark("账本导出与验链");
        Ok(CheckpointExports {
            roster,
            owners,
            audit_coverage: coverage,
        })
    }
    /// 先校验定格（排空、结构、业务引用），再核回执；窗口里核过的加上这里核的，只有实际核验成功才累计。
    fn verify_audit(&mut self) -> Result<AuditCoverageProof> {
        let mut checked = self.checked;
        if let Some(control) = &mut self.control {
            control
                .validate()
                .map_err(|_| unavailable(Owner::Control))?;
            add(
                &mut checked,
                control
                    .verify_audit(&self.audit)
                    .map_err(|_| unavailable(Owner::Control))?
                    .checked(),
            )?;
        }
        self.auth.validate().map_err(|_| unavailable(Owner::Auth))?;
        add(
            &mut checked,
            self.auth
                .verify_audit(&self.audit)
                .map_err(|_| unavailable(Owner::Auth))?
                .checked(),
        )?;
        match (&mut self.agents, &self.agent_audit) {
            (Some(agent), Some(audit)) => {
                agent
                    .validate()
                    .map_err(|_| unavailable(Owner::AgentStore))?;
                add(
                    &mut checked,
                    agent
                        .verify_audit(audit)
                        .map_err(|_| unavailable(Owner::AgentStore))?
                        .checked(),
                )?;
            }
            (None, None) => {}
            _ => return Err(unavailable(Owner::AgentStore)),
        }
        check_deadline(self.deadline, Owner::Audit)?;
        Ok(AuditCoverageProof {
            acked_events_checked: checked,
            acked_events_matched: checked,
            offset_comparison_only: false,
        })
    }
}
