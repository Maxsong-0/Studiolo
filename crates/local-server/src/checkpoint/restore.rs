//! 新根恢复的编排：按已核验清单里的名单，逐个走每个 owner 自己的恢复入口（T-212，设计见 vault
//! `04-协调/2026-11-16-T-212-新根恢复重新打开设计.md`、ADR-035）。
//!
//! 顺序：放置组件 → Broker 重绑定（lineage 记下恢复标识）→ 核两条账本的链尾 → 黑板三件失效（Control 撤销
//! 全部活动 attempt，策略库全局吊销，AgentStore 吊销全部类型 token、实例在策略库记实例吊销）→ 名单里有 Bridge
//! 的记「同步留在关闭」→ 最后 Auth 撤权（旧会话、L3、恢复码全部失效，身份版本变大，两条恢复事件投进账本）。
//! Auth 撤权放最后：主服务的启动闸门（`startup/restore_gate.rs`）只认「这次恢复的 Auth 撤权做完了」，
//! 它做完就说明前面每一步都做完了。
//!
//! 拒绝都发生在写任何东西之前；写了之后任何一步失败都停在「新根起不来」，由人看过再接手。接手要接在同一份
//! 恢复记录上：Broker 重绑定还没开始的，已放好的组件逐个按已核验视图重核；某个 owner 的失效已经开始过的，
//! 它的库已经改过，不再按封存水位核，只做幂等的失效和事后核对（`owners.rs`）。
//! 这里不建空库、不补默认 owner、不做最终切换。
mod owners;

use super::export::{AGENT_AUDIT, AGENTS, AUDIT, AUTH, CANONICAL, CONTROL, POLICY, WRITER_DATA};
use owners::{History, Stage};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{RestoreInvalidation, SessionConfig, SessionManager, TotpSeed};
use studiolo_backup::{
    checkpoint::staging::{Placement, RestoredCheckpoint},
    recovery::{
        kit::{PreRestoreChecks, RecoveryGrant},
        transcript::RecoveryStep,
    },
};
use studiolo_core::{
    VaultUuid,
    checkpoint::{AuditChainCutoff, CheckpointOwner, OwnerAssembly, OwnerWatermark},
};
use studiolo_vault_broker::{
    NewRootRebind, RestoreLineage, read_restore_lineage, rebind_new_roots,
};

/// 一个要放到新根里的组件：封存里的目录 → 新根下面的固定位置；位置不由调用方给。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    component: &'static str,
    /// 放在新根下面哪个组件里；`None` 是新根本身。接入方账本放在身份库目录里，和 agent-server 的布局一样。
    parent: Option<&'static str>,
    name: &'static str,
}
impl Slot {
    const fn at(component: &'static str, name: &'static str) -> Self {
        Self {
            component,
            parent: None,
            name,
        }
    }
    fn parent_path(&self, new_root: &Path) -> PathBuf {
        self.parent
            .map_or_else(|| new_root.to_owned(), |parent| new_root.join(parent))
    }
}
const HUMAN: [Slot; 4] = [
    Slot::at(CANONICAL, "vault"),
    Slot::at(WRITER_DATA, "data"),
    Slot::at(AUDIT, "audit"),
    Slot::at(AUTH, "auth"),
];
const CONTROL_SLOT: Slot = Slot::at(CONTROL, "control");
const AGENT_SLOT: Slot = Slot::at(AGENTS, "agent");
const AGENT_AUDIT_SLOT: Slot = Slot {
    component: AGENT_AUDIT,
    parent: Some("agent"),
    name: "audit",
};
const POLICY_SLOT: Slot = Slot::at(POLICY, "policy");

/// 一次新根恢复的输入。
pub struct NewRootRestore<'a> {
    /// 已经只读核验过的取回副本。
    pub restored: &'a RestoredCheckpoint,
    /// 新根：已经存在、属于当前账号、权限 0700 的目录；组件放在它下面的固定名字里。
    pub new_root: &'a Path,
    /// 本次恢复的标识：写进恢复记录、Broker lineage 和 Auth 撤权记录，三处是同一个（1～55 个字母、数字或 `-`）。
    pub restore_id: &'a str,
    /// 上一次恢复的 lineage 标识；第一次恢复为 `None`。
    pub parent_lineage: Option<String>,
    /// 明确接手上一次中途失败留下的新根。为 false 时新根里已有任何一个固定名字都拒绝；为 true 时必须接在
    /// 上一次那份恢复记录上（里面有这个恢复标识的放置意图）。
    pub resume_interrupted: bool,
    /// 恢复写入之前人已经确认过的前置检查。
    pub checks: PreRestoreChecks,
    /// 整次恢复共用的截止时间。
    pub deadline: Instant,
    /// 主人的会话配置（来自私有配置），打开恢复出来的 Auth 用。
    pub sessions: SessionConfig,
    /// 主服务用的那个 TOTP 种子：从生产机的凭据槽或主人密码管理器取，经 stdin 交进来，不进命令行参数、
    /// 不进恢复记录。撤权前照离线恢复工具的顺序挂上因子。
    pub totp_seed: TotpSeed,
}

/// 固定的拒绝类别；不带路径、键和底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreRefusal {
    /// 恢复记录不是这个 checkpoint 的、还没取得写入授权、写不进去，或者和「新开 / 接手」对不上：
    /// 新开一次恢复要用干净的记录，接手要接在上一次那份记录上。
    Transcript,
    /// 前置检查没有全部确认。
    ChecksIncomplete,
    /// 名单或水位不成套：必有的 owner 缺席、水位缺项或和名单对不上、有 AgentStore 却没有策略库
    /// （实例吊销记在策略库里）。写任何东西之前就拒绝。
    Roster,
    /// 新根里已经有固定名字，而调用方没有声明接手上一次的恢复。
    AlreadyPlaced,
    /// 新根里留着上一次放置中途失败的 `<名字>.restoring`：由人看过、清掉再来。写任何东西之前就拒绝，
    /// 声明接手也一样。
    CleanupRequired,
    /// 封存内容没能逐字节放到新根，或者接手时已放好的组件和已核验视图对不上。
    Placement,
    /// Broker 的新根重绑定没有完成。
    Rebind,
    /// 人侧账本或接入方账本打不开，或者链尾和清单里的水位对不上（水位之后只许有这次恢复 Auth 撤权自己记的事件）。
    Audit,
    /// 这个 owner 的失效没有做完：库打不开、和清单水位不符、失效没做成或事后核对不过。新根放好了，
    /// 启动闸门不放行；由人看过之后声明接手再来。
    Owner(CheckpointOwner),
    /// Auth 撤权没有完成：打不开、因子挂不上、撤权失败、身份版本没有变大，或两条恢复事件没投进账本。
    /// 这时新根已经放好，但启动闸门不放行；由人看过之后声明接手再来一次。
    Auth,
}
impl std::fmt::Display for RestoreRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "新根恢复未完成：{self:?}")
    }
}
impl std::error::Error for RestoreRefusal {}

/// 做到「组件已放好、Broker 已重绑定、链尾对上、各 owner 失效完成、Auth 撤权完成」时的回执；不是服务可以
/// 上线的证明，还差人核对之后的最终切换。
#[derive(Debug)]
pub struct RestoreReport {
    /// Broker 重绑定完成后的 lineage（保留下来的 Vault 身份、配对标识、这次的恢复标识）。
    pub lineage: RestoreLineage,
    /// 这次有没有跳过放置：接手上一次时已经在新根里的组件不重写。
    pub reused_placement: bool,
    /// 放进新根、做过失效的 owner（按冻结顺序）；Bridge 不在里面，它的运行状态不恢复。
    pub restored_owners: Vec<CheckpointOwner>,
    /// 名单里有 Bridge：双向同步留在关闭，只能按引导流程从零重新打开。
    pub sync_left_closed: bool,
    /// Auth 撤权的结果（新的身份版本和三个计数，不带任何凭据）。
    pub invalidation: RestoreInvalidation,
}

/// 清单里读出来、恢复要对着核的事实。
struct Plan {
    vault_uuid: VaultUuid,
    pair_id: VaultUuid,
    audit: AuditChainCutoff,
    agent_audit: Option<AuditChainCutoff>,
    control: Option<OwnerWatermark>,
    agents: Option<OwnerWatermark>,
    policy: Option<OwnerWatermark>,
    bridge: bool,
}
impl Plan {
    /// 要放的组件，按放置顺序；名单里缺席的 owner 不出现。
    fn slots(&self) -> Vec<Slot> {
        let mut slots = HUMAN.to_vec();
        if self.control.is_some() {
            slots.push(CONTROL_SLOT);
        }
        if self.agents.is_some() {
            slots.extend([AGENT_SLOT, AGENT_AUDIT_SLOT]);
        }
        if self.policy.is_some() {
            slots.push(POLICY_SLOT);
        }
        slots
    }
    fn restored_owners(&self) -> Vec<CheckpointOwner> {
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

/// 把一份已核验的取回副本恢复到新根。
///
/// 顺序：核恢复记录和前置检查 → 按名单定能不能恢复（不能就在这里拒绝，还没写任何东西）→ 新根里有
/// `.restoring` 半成品就拒绝 → 核「新开 / 接手」和恢复记录对得上 → 接手且 Broker 重绑定还没开始的，重核已放好的
/// 组件 → 逐字节核对着放置组件 → Broker 重绑定 → 核两条账本链尾 → 各 owner 失效 → Auth 撤权。每一步都记进
/// 恢复记录。成功返回后只剩人核对之后的最终切换，不在这里做。
pub fn restore_into_new_root(
    grant: &mut RecoveryGrant,
    request: NewRootRestore<'_>,
) -> Result<RestoreReport, RestoreRefusal> {
    let checkpoint = &request.restored.reference().checkpoint_id;
    if &grant.transcript.checkpoint_id != checkpoint || grant.transcript.authorize_write().is_err()
    {
        return Err(RestoreRefusal::Transcript);
    }
    if !request.checks.is_complete() {
        return Err(RestoreRefusal::ChecksIncomplete);
    }
    let plan = plan(request.restored)?;
    let slots = plan.slots();
    let mut placed = BTreeSet::new();
    for slot in &slots {
        // 半成品放置那一步一定会拒；先在这里拒，免得恢复记录已经追加、前几个组件已经放好（D2-F3）。
        let (present, interrupted) = state(slot, request.new_root)?;
        if interrupted {
            return Err(RestoreRefusal::CleanupRequired);
        }
        if present {
            placed.insert(slot.name);
        }
    }
    if !placed.is_empty() && !request.resume_interrupted {
        return Err(RestoreRefusal::AlreadyPlaced);
    }
    // 一份恢复记录只管一次恢复：接手必须接在记过这次放置意图的那份上，新开一次必须用干净的记录（D2-F2）。
    let history = History::of(&grant.transcript, request.restore_id);
    let continuing = if request.resume_interrupted {
        history.started
    } else {
        !history.any_started
    };
    if !continuing {
        return Err(RestoreRefusal::Transcript);
    }
    let data_root = request.new_root.join("data");
    if request.resume_interrupted && untouched(&placed, &data_root)? {
        // Broker 重绑定还没开始：已放好的组件一个字节都还没动过，逐个按已核验视图重核（D2-F2）。
        for slot in slots.iter().filter(|slot| placed.contains(slot.name)) {
            let nested: Vec<&str> = slots
                .iter()
                .filter(|inner| inner.parent == Some(slot.name))
                .map(|inner| inner.name)
                .collect();
            request
                .restored
                .verify_placed(
                    slot.component,
                    &slot.parent_path(request.new_root),
                    slot.name,
                    &nested,
                    request.deadline,
                )
                .map_err(|_| RestoreRefusal::Placement)?;
        }
    }
    record(
        grant,
        RecoveryStep::PreRestoreChecksCompleted,
        serde_json::json!({ "checks": request.checks }),
    )?;
    record(
        grant,
        RecoveryStep::PhysicalRebindIntent,
        serde_json::json!({
            "restore": request.restore_id,
            "reused_components": placed.len(),
            "resumed": request.resume_interrupted,
        }),
    )?;

    for slot in &slots {
        let placement = request
            .restored
            .extract(
                slot.component,
                &slot.parent_path(request.new_root),
                slot.name,
                request.deadline,
            )
            .map_err(|_| RestoreRefusal::Placement)?;
        if placement == Placement::AlreadyPresent && !request.resume_interrupted {
            return Err(RestoreRefusal::AlreadyPlaced);
        }
    }

    let lineage = rebind(&request, &plan, &data_root)?;
    record(
        grant,
        RecoveryStep::PhysicalRebindOutcome,
        serde_json::json!({ "restore": request.restore_id }),
    )?;

    let ledger = SqliteAuditLedger::open_existing(&request.new_root.join("audit"))
        .map_err(|_| RestoreRefusal::Audit)?;
    // 水位之后多出的条目只能是这次恢复 Auth 撤权自己记的，而且只在它已经开始过时才可能有。
    let auth_begun = history.begun(Stage::Auth).then_some(request.restore_id);
    owners::check_tail(&ledger, &plan.audit, auth_begun).map_err(|()| RestoreRefusal::Audit)?;
    if let Some(cutoff) = &plan.agent_audit {
        // 恢复从不往接入方账本里写，所以那边水位之后一条都不能多。
        let agent = SqliteAuditLedger::open_existing(&request.new_root.join("agent/audit"))
            .map_err(|_| RestoreRefusal::Audit)?;
        owners::check_tail(&agent, cutoff, None).map_err(|()| RestoreRefusal::Audit)?;
    }

    let now = jiff::Timestamp::now();
    let reason = format!("新根恢复 {}", request.restore_id);
    let context = owners::Context {
        new_root: request.new_root,
        restore_id: request.restore_id,
        now: &now,
        reason: &reason,
        deadline: request.deadline,
    };
    if let Some(expected) = &plan.control {
        owners::run_stage(grant, &history, &context, Stage::Control, |fresh| {
            owners::control(&context, plan.vault_uuid, expected, fresh)
        })?;
    }
    if let Some(expected) = &plan.policy {
        owners::run_stage(grant, &history, &context, Stage::Policy, |fresh| {
            owners::policy(&context, expected, fresh)
        })?;
    }
    if let Some(expected) = &plan.agents {
        owners::run_stage(grant, &history, &context, Stage::Agents, |fresh| {
            owners::agents(&context, expected, fresh)
        })?;
    }
    if plan.bridge && !history.recorded(RecoveryStep::SyncLeftClosed) {
        // Bridge 绑着旧根的物理身份，这一版不恢复它的运行状态，目录也不放；同步按引导流程从零重新打开。
        record(
            grant,
            RecoveryStep::SyncLeftClosed,
            serde_json::json!({ "restore": request.restore_id }),
        )?;
    }
    let invalidation = owners::run_stage(grant, &history, &context, Stage::Auth, |_| {
        owners::auth(
            request.new_root,
            request.sessions,
            request.totp_seed,
            request.restore_id,
            Arc::new(ledger),
        )
    })?;
    Ok(RestoreReport {
        lineage,
        reused_placement: !placed.is_empty(),
        restored_owners: plan.restored_owners(),
        sync_left_closed: plan.bridge,
        invalidation,
    })
}

/// 组件在新根里的状态：（已放好，上一次放置中途失败留下的 `.restoring` 还在）。放在别的组件里面的，
/// 外层还没放好就当两样都没有。
fn state(slot: &Slot, new_root: &Path) -> Result<(bool, bool), RestoreRefusal> {
    if let Some(parent) = slot.parent
        && !RestoredCheckpoint::placed(new_root, parent).map_err(|_| RestoreRefusal::Placement)?
    {
        return Ok((false, false));
    }
    let parent = slot.parent_path(new_root);
    let interrupted = RestoredCheckpoint::interrupted(&parent, slot.name)
        .map_err(|_| RestoreRefusal::Placement)?;
    let present =
        RestoredCheckpoint::placed(&parent, slot.name).map_err(|_| RestoreRefusal::Placement)?;
    Ok((present, interrupted))
}

/// 接手时新根是不是还一点没动过：写者数据根还没放好，或者放好了但上面没有 lineage（Broker 重绑定还没开始）。
fn untouched(placed: &BTreeSet<&str>, data_root: &Path) -> Result<bool, RestoreRefusal> {
    if !placed.contains("data") {
        return Ok(true);
    }
    Ok(read_restore_lineage(data_root)
        .map_err(|_| RestoreRefusal::Rebind)?
        .is_none())
}

/// Broker 重绑定。接手上一次、重绑定那一步已经做完的，只认同一次恢复（同一 checkpoint、同一恢复标识、同一
/// Vault 身份和配对标识）的 lineage，不再重绑一遍。
fn rebind(
    request: &NewRootRestore<'_>,
    plan: &Plan,
    data_root: &Path,
) -> Result<RestoreLineage, RestoreRefusal> {
    let checkpoint = &request.restored.reference().checkpoint_id;
    if let Some(done) = read_restore_lineage(data_root).map_err(|_| RestoreRefusal::Rebind)?
        && done.is_completed()
    {
        if !request.resume_interrupted
            || done.restore_id.as_deref() != Some(request.restore_id)
            || done.checkpoint_id != checkpoint.as_str()
            || done.vault_uuid != plan.vault_uuid
            || done.pair_id != plan.pair_id
        {
            return Err(RestoreRefusal::Rebind);
        }
        return Ok(done);
    }
    let receipt = rebind_new_roots(&NewRootRebind {
        new_vault_root: &request.new_root.join("vault"),
        new_data_root: data_root,
        expected_vault_uuid: plan.vault_uuid,
        expected_pair_id: plan.pair_id,
        checkpoint_id: checkpoint.as_str().to_owned(),
        restore_id: request.restore_id,
        parent_lineage: request.parent_lineage.clone(),
        resume_interrupted: request.resume_interrupted,
    })
    .map_err(|_| RestoreRefusal::Rebind)?;
    if receipt.vault_uuid != plan.vault_uuid || receipt.pair_id != plan.pair_id {
        return Err(RestoreRefusal::Rebind);
    }
    Ok(receipt.lineage)
}

/// 只看已核验的清单，定这份封存这一版能不能恢复；不碰磁盘。
fn plan(restored: &RestoredCheckpoint) -> Result<Plan, RestoreRefusal> {
    let manifest = restored.manifest();
    let mut present = BTreeSet::new();
    for (who, assembly) in manifest.roster.entries() {
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
    for watermark in &manifest.watermarks {
        match watermark {
            OwnerWatermark::Broker {
                vault_uuid,
                pair_id,
                unresolved: false,
                ..
            } => broker = Some((*vault_uuid, *pair_id)),
            OwnerWatermark::Audit {
                last_offset,
                last_hash,
                agent_ledger,
            } if *last_offset > 0 => {
                audit = Some(AuditChainCutoff {
                    last_offset: *last_offset,
                    last_hash: last_hash.clone(),
                });
                agent_audit.clone_from(agent_ledger);
            }
            OwnerWatermark::Auth { .. } => auth = true,
            OwnerWatermark::Control { .. } => control = Some(watermark.clone()),
            OwnerWatermark::AgentStore { .. } => agents = Some(watermark.clone()),
            OwnerWatermark::Policy { .. } => policy = Some(watermark.clone()),
            // 清单校验已经要求持久门禁关着；这里再守一次，开着的不恢复。
            OwnerWatermark::Bridge {
                durable_gate_closed: true,
                ..
            } => bridge = true,
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

fn record(
    grant: &mut RecoveryGrant,
    step: RecoveryStep,
    payload: serde_json::Value,
) -> Result<(), RestoreRefusal> {
    grant
        .transcript
        .append(step, payload)
        .map(drop)
        .map_err(|_| RestoreRefusal::Transcript)
}

/// 启动闸门的拒绝：这是恢复出来的新根，Auth 撤权还没做完（或看不清是不是）。不带路径和底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RevocationPending;

/// 新根恢复的启动闸门（T-212），主服务打开 Auth 之后、HTTP 就绪检查之前调：跟着数据走，不跟着路径走。
///
/// 写者数据根上有恢复 lineage（Broker 重绑定时写下的），就要求 Auth 里有同一恢复标识、两条恢复事件都已投进
/// 账本的撤权记录；第 1 版 lineage 没有恢复标识，一律当「撤权没做」；lineage 读不出来也不放行。没有 lineage
/// 的是原生库，照常启动。改配置、换目录名都绕不过：lineage 和撤权记录都在数据里。Broker 重绑定没完成
/// （lineage 停在 `Rebinding`）由 Broker 打开时自己拒绝，不归这里。编排里 Auth 撤权是最后一步，它做完就说明
/// 黑板三件的失效也都做完了。
///
/// 已知限制：不带写者数据根启动（只读模式）时没有 lineage 可查，这道闸门管不到；恢复出来的新根必须按生产
/// 配置带写者数据根启动。agent-server 是另一个程序，没有这道闸门：最终切换之前不要把它指到新根。
pub fn require_restore_revoked(
    writer_data: Option<&Path>,
    sessions: &SessionManager,
) -> Result<(), RevocationPending> {
    let Some(data) = writer_data else {
        return Ok(());
    };
    let Some(lineage) = read_restore_lineage(data).map_err(|_| RevocationPending)? else {
        return Ok(());
    };
    let restore_id = lineage.restore_id.as_deref().ok_or(RevocationPending)?;
    if sessions
        .restore_completed(restore_id)
        .map_err(|_| RevocationPending)?
    {
        Ok(())
    } else {
        Err(RevocationPending)
    }
}
