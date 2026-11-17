//! 恢复动手之前的两个判断，都不碰磁盘，用例直接喂值测：只看已核验清单定这一份能不能恢复（[`plan_from`]），
//! 和新根 data 里读到的 lineage 对这次恢复意味着什么（[`classify`]）。
use super::RestoreRefusal;
use std::collections::BTreeSet;
use studiolo_backup::checkpoint::types::OwnerRoster;
use studiolo_core::{
    VaultUuid,
    checkpoint::{AuditChainCutoff, CheckpointOwner, OwnerAssembly, OwnerWatermark},
};
use studiolo_vault_broker::{LineageState, RestoreLineage};

/// 清单里读出来、恢复要对着核的事实。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) vault_uuid: VaultUuid,
    pub(crate) pair_id: VaultUuid,
    pub(crate) audit: AuditChainCutoff,
    pub(crate) agent_audit: Option<AuditChainCutoff>,
    pub(crate) control: Option<OwnerWatermark>,
    pub(crate) agents: Option<OwnerWatermark>,
    pub(crate) policy: Option<OwnerWatermark>,
    /// 名单里有 Bridge（持久门禁关着）：它的运行状态不恢复，同步留在关闭。
    pub(crate) bridge: bool,
}
impl Plan {
    /// 放进新根、要做失效的 owner（按冻结顺序）；Bridge 不在里面。
    pub(crate) fn restored_owners(&self) -> Vec<CheckpointOwner> {
        CheckpointOwner::FREEZE_ORDER
            .into_iter()
            .filter(|owner| match owner {
                CheckpointOwner::Control => self.control.is_some(),
                CheckpointOwner::Bridge => false,
                CheckpointOwner::AgentStore => self.agents.is_some(),
                CheckpointOwner::Policy => self.policy.is_some(),
                CheckpointOwner::Broker | CheckpointOwner::Auth | CheckpointOwner::Audit => true,
            })
            .collect()
    }
}

/// 只看已核验清单里的名单和水位，定这份封存这一版能不能恢复。
///
/// 拒绝（[`RestoreRefusal::Roster`]）：Broker、Auth、Audit 缺席；水位缺项、多出、和名单对不上；Broker 还有
/// 没处理完的；人侧账本是空的；Bridge 的持久门禁开着；有 AgentStore 却没有它的接入方账本或没有策略库（实例
/// 吊销记在策略库里），或者没有 AgentStore 却有接入方账本。
pub(crate) fn plan_from(
    roster: &OwnerRoster,
    watermarks: &[OwnerWatermark],
) -> Result<Plan, RestoreRefusal> {
    let mut present = BTreeSet::new();
    for (who, assembly) in roster.entries() {
        let required = matches!(
            who,
            CheckpointOwner::Broker | CheckpointOwner::Auth | CheckpointOwner::Audit
        );
        match assembly {
            OwnerAssembly::Present => {
                present.insert(who);
            }
            OwnerAssembly::Absent if required => return Err(RestoreRefusal::Roster),
            OwnerAssembly::Absent => {}
        }
    }
    let mut broker = None;
    let mut audit = None;
    let mut agent_audit = None;
    let mut auth = false;
    let mut control = None;
    let mut agents = None;
    let mut policy = None;
    let mut bridge = false;
    for watermark in watermarks {
        match watermark {
            OwnerWatermark::Broker {
                vault_uuid,
                pair_id,
                unresolved: false,
                ..
            } if broker.is_none() => broker = Some((*vault_uuid, *pair_id)),
            OwnerWatermark::Audit {
                last_offset,
                last_hash,
                agent_ledger,
            } if *last_offset > 0 && audit.is_none() => {
                audit = Some(AuditChainCutoff {
                    last_offset: *last_offset,
                    last_hash: last_hash.clone(),
                });
                agent_audit.clone_from(agent_ledger);
            }
            OwnerWatermark::Auth { .. } if !auth => auth = true,
            OwnerWatermark::Control { .. } if control.is_none() => {
                control = Some(watermark.clone());
            }
            OwnerWatermark::AgentStore { .. } if agents.is_none() => {
                agents = Some(watermark.clone());
            }
            OwnerWatermark::Policy { .. } if policy.is_none() => policy = Some(watermark.clone()),
            // 清单校验已经要求持久门禁关着；这里再守一次，开着的不恢复。
            OwnerWatermark::Bridge {
                durable_gate_closed: true,
                ..
            } if !bridge => bridge = true,
            // 门禁开着、Broker 有没处理完的、账本是空的、同一个 owner 出现两次：都不恢复。
            _ => return Err(RestoreRefusal::Roster),
        }
    }
    // 名单和水位一一对上；有 AgentStore 才有第二条账本；AgentStore 的实例吊销记在策略库里，没有策略库不恢复。
    let consistent = control.is_some() == present.contains(&CheckpointOwner::Control)
        && agents.is_some() == present.contains(&CheckpointOwner::AgentStore)
        && policy.is_some() == present.contains(&CheckpointOwner::Policy)
        && bridge == present.contains(&CheckpointOwner::Bridge)
        && agent_audit.is_some() == agents.is_some()
        && (agents.is_none() || policy.is_some());
    let ((vault_uuid, pair_id), audit) = broker
        .zip(audit)
        .filter(|_| auth && consistent)
        .ok_or(RestoreRefusal::Roster)?;
    Ok(Plan {
        vault_uuid,
        pair_id,
        audit,
        agent_audit,
        control,
        agents,
        policy,
        bridge,
    })
}

/// 新根 data 里读到的 lineage 对这次恢复意味着什么。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// 这次的重绑定还没开始：data 里没有 lineage，或者只有被封存的那个根自己的（那个根是恢复出来的：lineage
    /// 已完成、是另一个 checkpoint 的）。`parent` 是那个根的恢复标识，记进这次的 lineage 当上一环。
    Fresh { parent: Option<String> },
    /// 这次的重绑定中途断了：同一 checkpoint、同一恢复标识，停在重绑定中。`parent` 照第一次记的。
    Interrupted { parent: Option<String> },
    /// 这次的重绑定已经做完。
    Done(Box<RestoreLineage>),
}

/// 判断 data 里读到的 lineage（[`Found`]）。和这次恢复对不上的一律拒绝（[`RestoreRefusal::Rebind`]）：Vault 身份
/// 或配对标识不同；别的 checkpoint 的却没完成或没有恢复标识（停在重绑定中途的根起不了服务、不会被封存；第 1 版
/// 没有恢复标识、接不成链）；同一个 checkpoint 的却是别的恢复标识（只能是这次恢复自己写的）。
///
/// 被封存的根自己的 lineage 在这里只当上一环，不当成这次的：Broker 的封存把 data 整棵复制，恢复出来的根的
/// 封存里一定带着它（Codex 第二意见第 5 条）。这次的恢复标识是不是以前用过，由 Broker 重绑定按 lineage 记的
/// 那一串拒绝。
pub(crate) fn classify(
    found: Option<RestoreLineage>,
    checkpoint: &str,
    restore_id: &str,
    vault: VaultUuid,
    pair: VaultUuid,
) -> Result<Found, RestoreRefusal> {
    let Some(lineage) = found else {
        return Ok(Found::Fresh { parent: None });
    };
    if lineage.vault_uuid != vault || lineage.pair_id != pair {
        return Err(RestoreRefusal::Rebind);
    }
    if lineage.checkpoint_id != checkpoint {
        return match lineage.restore_id.clone() {
            Some(parent) if lineage.is_completed() => Ok(Found::Fresh {
                parent: Some(parent),
            }),
            _ => Err(RestoreRefusal::Rebind),
        };
    }
    if lineage.restore_id.as_deref() != Some(restore_id) {
        return Err(RestoreRefusal::Rebind);
    }
    if lineage.is_completed() {
        return Ok(Found::Done(Box::new(lineage)));
    }
    if lineage.state == LineageState::Rebinding {
        return Ok(Found::Interrupted {
            parent: lineage.parent_lineage,
        });
    }
    Err(RestoreRefusal::Rebind)
}
