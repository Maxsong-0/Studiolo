//! 冻结窗口放开之后（BUG-045）：各库只剩定格在冻结那一刻的读连接，写者只剩两棵树的文件句柄，
//! 业务照常读写。这里核剩下的回执、逐项导出、登记来源、封存暂存区；读到的全是冻结那一刻。
//! 没有屏障可以放开：丢弃只关定格的连接和句柄，失败不出票，留下的暂存由暂存层回收。
use super::{
    CheckpointExports, CheckpointTimes, OwnerExportPaths, PolicyParticipant, Result, add,
    check_deadline, unavailable,
};
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
impl ReleasedOwners {
    /// 逐项导出，过滤视图与本地清单原子封存、重开复验；暂存区的截止时间不能晚于放开时给的截止时间。
    pub fn seal(
        &mut self,
        area: StagingArea,
        catalog: impl FnOnce(
            &Path,
            &CheckpointExports,
        ) -> std::result::Result<SourceCatalog, StagingError>,
        mut manifest: ManifestInputs,
        times: CheckpointTimes,
    ) -> std::result::Result<SealedCheckpoint, StagingError> {
        let started = Instant::now();
        if self.paths.directories() != OwnerExportPaths::under(area.export_root()).directories()
            || area.deadline() > self.deadline
        {
            return Err(StagingError::UnsafeRoot);
        }
        let exports = self.export().map_err(|_| StagingError::Unavailable)?;
        // 只在真实副本已经形成之后计算当前登记；不得按路径补默认标签。
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
                now: InstantMillis::from_millis(now),
            },
        )?;
        check_deadline(self.deadline, Owner::Audit).map_err(|_| StagingError::Expired)?;
        Ok(sealed)
    }
    /// 先核剩下的回执（登录、控制、接入方库，都读各自定格的库和定格的账本），再按固定顺序逐项导出，
    /// 汇总两个独立审计截止点。账本的整本验链在它自己的导出里、在副本上做。
    pub fn export(&mut self) -> Result<CheckpointExports> {
        let coverage = self.verify_audit()?;
        let p = &self.paths;
        let mut owners = vec![
            self.broker
                .export(&p.vault, &p.writer, self.deadline)
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
    /// 窗口里核过的加上这里核的；只有实际核验成功才累计。
    fn verify_audit(&mut self) -> Result<AuditCoverageProof> {
        let mut checked = self.checked;
        if let Some(control) = &mut self.control {
            add(
                &mut checked,
                control
                    .verify_audit(&self.audit)
                    .map_err(|_| unavailable(Owner::Control))?
                    .checked(),
            )?;
        }
        add(
            &mut checked,
            self.auth
                .verify_audit(&self.audit)
                .map_err(|_| unavailable(Owner::Auth))?
                .checked(),
        )?;
        match (&mut self.agents, &self.agent_audit) {
            (Some(agent), Some(audit)) => add(
                &mut checked,
                agent
                    .verify_audit(audit)
                    .map_err(|_| unavailable(Owner::AgentStore))?
                    .checked(),
            )?,
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
