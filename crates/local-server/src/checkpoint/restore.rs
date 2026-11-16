//! 新根恢复的编排：按已核验清单里的名单，逐个走每个 owner 自己的恢复入口（T-212，设计见 vault
//! `04-协调/2026-11-16-T-212-新根恢复重新打开设计.md`、ADR-035）。
//!
//! 顺序：放置组件 → Broker 重绑定（lineage 记下恢复标识和这份数据之前用过的全部恢复标识）→ 核两条账本的链尾 →
//! 黑板三件失效（Control 撤销全部活动 attempt，策略库全局吊销，AgentStore 吊销全部类型 token、实例在策略库记
//! 实例吊销）→ 名单里有 Bridge 的记「同步留在关闭」→ 最后 Auth 撤权（旧会话、L3、恢复码全部失效，身份版本变大，
//! 两条恢复事件投进账本）。Auth 撤权放最后：主服务的启动闸门（`startup/restore_gate.rs`）只认「这次恢复的 Auth
//! 撤权做完了」，它做完就说明前面每一步都做完了。
//!
//! 带凭据的库（Auth、身份库）先放在暂用名（`auth.held`、`agent.held`）下，它的 owner 把旧凭据全部作废之后才改成
//! 正式名：之前按正式路径谁都打不开，包括不带写者数据根、不经启动闸门的只读模式和 agent-server（Codex 第二意见
//! 第 2 条）。启动闸门照旧是写者模式的那一道。
//!
//! 拒绝都尽量发生在写任何东西之前；写了之后任何一步失败都停在「新根起不来」，由人看过再接手。接手要接在同一份
//! 恢复记录上：Broker 重绑定还没开始的，已放好的组件逐个按已核验视图重核；某个 owner 的失效已经开始过的，
//! 它的库已经改过，不再按封存水位核，只做幂等的失效和事后核对（`owners.rs`）。
//! 每追加一条恢复记录，先交给调用方的去处（命令行写到标准输出并刷新），交出去了才做那一步；交不出去就停，
//! 人手里那份记录里没有的步骤一步都不做（[`Recorder`]）。
//! 这里不建空库、不补默认 owner；最终切换在 `cutover.rs`，由人核对之后另外执行。
mod cutover;
mod owners;
mod plan;

use super::export::{AGENT_AUDIT, AGENTS, AUDIT, AUTH, CANONICAL, CONTROL, POLICY, WRITER_DATA};
use owners::{History, Stage};
pub(crate) use plan::{Found, Plan, classify, plan_from};
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
        transcript::{RecoveryStep, RecoveryTranscript, TranscriptEntry},
    },
};
use studiolo_core::checkpoint::CheckpointOwner;
use studiolo_vault_broker::{
    NewRootRebind, RestoreError, RestoreLineage, read_restore_lineage, rebind_new_roots,
    valid_restore_id,
};

pub use cutover::{Cutover, CutoverRefusal, begin_cutover, finish_cutover};

/// 恢复记录的去处：每追加一条就交给它一次，交不出去返回 `Err(())`。命令行把每条写到标准输出并刷新；
/// 进程内的调用方（用例）不需要另存时给一个什么都不做的。
pub type Journal<'a> = dyn FnMut(&TranscriptEntry) -> Result<(), ()> + 'a;

/// 恢复记录和它的去处，编排里每一步都经它记。
pub(super) struct Recorder<'a> {
    grant: &'a mut RecoveryGrant,
    journal: &'a mut Journal<'a>,
}
impl Recorder<'_> {
    /// 追加一条，交给去处；交出去了才返回，调用方这时才能做这一步。交不出去的那条从内存里撤掉：那一步没做，
    /// 内存里的记录要和人手里的一样，命令行按它判断新根写没写过。
    pub(super) fn record(
        &mut self,
        step: RecoveryStep,
        payload: serde_json::Value,
    ) -> Result<(), RestoreRefusal> {
        let transcript = &mut self.grant.transcript;
        let entry = transcript
            .append(step, payload)
            .map_err(|_| RestoreRefusal::Transcript)?;
        if (self.journal)(entry).is_err() {
            transcript.entries.pop();
            return Err(RestoreRefusal::Transcript);
        }
        Ok(())
    }
}

/// 一个要放到新根里的组件：封存里的目录 → 新根下面的固定位置；位置不由调用方给。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    component: &'static str,
    /// 放在哪个组件里面；`None` 是新根本身。接入方账本放在身份库目录里，和 agent-server 的布局一样。
    parent: Option<&'static Slot>,
    /// 正式名。
    name: &'static str,
    /// 带凭据的库先放在这个暂用名下，它的 owner 做完失效才改成正式名。
    held: Option<&'static str>,
}
impl Slot {
    const fn at(component: &'static str, name: &'static str) -> Self {
        Self {
            component,
            parent: None,
            name,
            held: None,
        }
    }
    const fn held(component: &'static str, name: &'static str, held: &'static str) -> Self {
        Self {
            component,
            parent: None,
            name,
            held: Some(held),
        }
    }
    /// 组件现在在父目录里的名字：正式名在就是正式名；带暂用名的、正式名还不在，就是暂用名（还没放的也按它放）。
    /// 两个名字同时在，说明有人在新根上另建了正式名，拒绝。
    fn current(&self, parent: &Path) -> Result<&'static str, RestoreRefusal> {
        let Some(held) = self.held else {
            return Ok(self.name);
        };
        let published = RestoredCheckpoint::placed(parent, self.name).map_err(placement)?;
        let holding = RestoredCheckpoint::placed(parent, held).map_err(placement)?;
        match (published, holding) {
            (true, true) => Err(RestoreRefusal::Placement),
            (true, false) => Ok(self.name),
            (false, _) => Ok(held),
        }
    }
    /// 父目录现在的路径（外层组件按它现在的名字）。
    fn parent_path(&self, new_root: &Path) -> Result<PathBuf, RestoreRefusal> {
        self.parent
            .map_or_else(|| Ok(new_root.to_owned()), |parent| parent.path(new_root))
    }
    /// 组件现在的路径（按 [`Self::current`]）。
    fn path(&self, new_root: &Path) -> Result<PathBuf, RestoreRefusal> {
        let parent = self.parent_path(new_root)?;
        let name = self.current(&parent)?;
        Ok(parent.join(name))
    }
    /// owner 做完失效之后，把暂用名改成正式名；已经是正式名（接手上一次时改过了）或者没有暂用名的什么都不做。
    fn publish(&self, new_root: &Path) -> Result<(), RestoreRefusal> {
        let Some(held) = self.held else {
            return Ok(());
        };
        let parent = self.parent_path(new_root)?;
        if self.current(&parent)? == held {
            RestoredCheckpoint::publish_held(&parent, held, self.name).map_err(placement)?;
        }
        Ok(())
    }
}
const AUTH_SLOT: Slot = Slot::held(AUTH, "auth", "auth.held");
const HUMAN: [Slot; 4] = [
    Slot::at(CANONICAL, "vault"),
    Slot::at(WRITER_DATA, "data"),
    Slot::at(AUDIT, "audit"),
    AUTH_SLOT,
];
const CONTROL_SLOT: Slot = Slot::at(CONTROL, "control");
const AGENT_SLOT: Slot = Slot::held(AGENTS, "agent", "agent.held");
const AGENT_AUDIT_SLOT: Slot = Slot {
    component: AGENT_AUDIT,
    parent: Some(&AGENT_SLOT),
    name: "audit",
    held: None,
};
const POLICY_SLOT: Slot = Slot::at(POLICY, "policy");

fn placement<E>(_: E) -> RestoreRefusal {
    RestoreRefusal::Placement
}

/// 一次新根恢复的输入。
pub struct NewRootRestore<'a> {
    /// 已经只读核验过的取回副本。
    pub restored: &'a RestoredCheckpoint,
    /// 新根：已经存在、属于当前账号、权限 0700 的目录；组件放在它下面的固定名字里。
    pub new_root: &'a Path,
    /// 本次恢复的标识：写进恢复记录、Broker lineage 和 Auth 撤权记录，三处是同一个（1～55 个字母、数字或 `-`）。
    /// 不能是这份数据以前用过的（被封存的根本身是恢复出来的，lineage 和 Auth 里都记着它们）。lineage 的
    /// 上一环由数据定：被封存的根是恢复出来的，就是它的恢复标识，不由调用方给。
    pub restore_id: &'a str,
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
    /// 恢复标识不合法（要 1～55 个字母、数字或 `-`，Broker lineage 和 Auth 撤权记录是同一个规则）。
    /// 写任何东西之前就拒绝。
    RestoreId,
    /// 这个恢复标识这份数据以前用过：Broker 按 lineage 记的那一串在重绑定之前拒绝，或者 Auth 撤权开始之前按
    /// Auth 自己的记录拒绝。旧记录还在，再用一次启动闸门会认它（Codex 第二意见第 1 条）。这时新根已经放好，但
    /// Auth 还在暂用名下、按正式路径打不开；由人删掉新根、换一个没用过的标识从头来，接手解决不了。
    RestoreIdUsed,
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
    /// 封存内容没能逐字节放到新根，接手时已放好的组件和已核验视图对不上，或者带凭据的库的正式名和暂用名同时
    /// 在（有人在新根上另建了正式名）。
    Placement,
    /// Broker 的新根重绑定没有完成，或者 data 里的 lineage 和这次恢复对不上（不是同一个 Vault、配对、
    /// checkpoint 或恢复标识）。
    Rebind,
    /// 人侧账本或接入方账本打不开，或者链尾和清单里的水位对不上（水位之后只许有这次恢复 Auth 撤权自己记的事件）。
    Audit,
    /// 这个 owner 的失效没有做完：库打不开、和清单水位不符、失效没做成、事后核对不过，或者做完之后改不成正式名。
    /// 新根放好了，启动闸门不放行；由人看过之后声明接手再来。
    Owner(CheckpointOwner),
    /// Auth 撤权没有完成：打不开、因子挂不上、撤权失败、身份版本没有变大、两条恢复事件没投进账本，或者做完之后
    /// 改不成正式名。这时新根已经放好，但启动闸门不放行；由人看过之后声明接手再来一次。
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
    /// Broker 重绑定完成后的 lineage（保留下来的 Vault 身份、配对标识、这次的恢复标识、之前用过的恢复标识）。
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

/// 要放的组件，按放置顺序；名单里缺席的 owner 不出现。外层组件排在放在它里面的组件前面。
fn slots(plan: &Plan) -> Vec<Slot> {
    let mut slots = HUMAN.to_vec();
    if plan.control.is_some() {
        slots.push(CONTROL_SLOT);
    }
    if plan.agents.is_some() {
        slots.extend([AGENT_SLOT, AGENT_AUDIT_SLOT]);
    }
    if plan.policy.is_some() {
        slots.push(POLICY_SLOT);
    }
    slots
}

/// 把一份已核验的取回副本恢复到新根。
///
/// 顺序：核恢复标识、恢复记录和前置检查 → 按名单定能不能恢复（不能就在这里拒绝，还没写任何东西）→ 新根里有
/// `.restoring` 半成品就拒绝 → 核「新开 / 接手」和恢复记录对得上 → 接手且这次的 Broker 重绑定还没开始的，重核
/// 已放好的组件 → 逐字节核对着放置组件（带凭据的放在暂用名下）→ Broker 重绑定 → 核两条账本链尾 → 各 owner
/// 失效 → Auth 撤权。每一步都记进恢复记录，每一条都先交给 `journal`。成功返回后只剩人核对之后的最终切换，
/// 不在这里做。
pub fn restore_into_new_root(
    grant: &mut RecoveryGrant,
    request: NewRootRestore<'_>,
    journal: &mut Journal<'_>,
) -> Result<RestoreReport, RestoreRefusal> {
    if !valid_restore_id(request.restore_id) {
        return Err(RestoreRefusal::RestoreId);
    }
    let checkpoint = &request.restored.reference().checkpoint_id;
    if &grant.transcript.checkpoint_id != checkpoint || grant.transcript.authorize_write().is_err()
    {
        return Err(RestoreRefusal::Transcript);
    }
    if !request.checks.is_complete() {
        return Err(RestoreRefusal::ChecksIncomplete);
    }
    let manifest = request.restored.manifest();
    let plan = plan_from(&manifest.roster, &manifest.watermarks)?;
    let slots = slots(&plan);
    // 按组件记（接入方账本和人侧账本的名字都叫 `audit`，按名字记会把两个当成一个，Codex 第二意见第 4 条）。
    let mut placed = BTreeSet::new();
    for slot in &slots {
        // 半成品放置那一步一定会拒；先在这里拒，免得恢复记录已经追加、前几个组件已经放好（D2-F3）。
        let (present, interrupted) = state(slot, request.new_root)?;
        if interrupted {
            return Err(RestoreRefusal::CleanupRequired);
        }
        if present {
            placed.insert(slot.component);
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
    let rebind_started = placed.contains(WRITER_DATA)
        && !matches!(found(&request, &plan, &data_root)?, Found::Fresh { .. });
    if request.resume_interrupted && !rebind_started {
        // 这次的 Broker 重绑定还没开始：已放好的组件一个字节都还没动过，逐个按已核验视图重核（D2-F2）。
        for slot in slots.iter().filter(|slot| placed.contains(slot.component)) {
            let parent = slot.parent_path(request.new_root)?;
            let name = slot.current(&parent)?;
            // 改正式名在它的 owner 失效之后，重绑定之前带暂用名的只会在暂用名下。
            if slot.held.is_some_and(|held| held != name) {
                return Err(RestoreRefusal::Placement);
            }
            let nested: Vec<&str> = slots
                .iter()
                .filter(|inner| inner.parent == Some(slot))
                .map(|inner| inner.name)
                .collect();
            request
                .restored
                .verify_placed(slot.component, &parent, name, &nested, request.deadline)
                .map_err(placement)?;
        }
    }
    let mut recorder = Recorder { grant, journal };
    recorder.record(
        RecoveryStep::PreRestoreChecksCompleted,
        serde_json::json!({ "checks": request.checks }),
    )?;
    recorder.record(
        RecoveryStep::PhysicalRebindIntent,
        serde_json::json!({
            "restore": request.restore_id,
            // 这次要恢复的 owner：最终切换按它核「名单里有的都失效了」。
            "owners": plan.restored_owners(),
            "reused_components": placed.len(),
            "resumed": request.resume_interrupted,
        }),
    )?;

    for slot in &slots {
        let parent = slot.parent_path(request.new_root)?;
        let name = slot.current(&parent)?;
        let outcome = request
            .restored
            .extract(slot.component, &parent, name, request.deadline)
            .map_err(placement)?;
        if outcome == Placement::AlreadyPresent && !request.resume_interrupted {
            return Err(RestoreRefusal::AlreadyPlaced);
        }
    }

    let lineage = rebind(&request, &plan, &data_root)?;
    recorder.record(
        RecoveryStep::PhysicalRebindOutcome,
        serde_json::json!({ "restore": request.restore_id }),
    )?;

    let ledger = Arc::new(
        SqliteAuditLedger::open_existing(&request.new_root.join("audit"))
            .map_err(|_| RestoreRefusal::Audit)?,
    );
    // 水位之后多出的条目只能是这次恢复 Auth 撤权自己记的，而且只在它已经开始过时才可能有。
    let auth_begun = history.begun(Stage::Auth).then_some(request.restore_id);
    owners::check_tail(&ledger, &plan.audit, auth_begun).map_err(|()| RestoreRefusal::Audit)?;
    if let Some(cutoff) = &plan.agent_audit {
        // 恢复从不往接入方账本里写，所以那边水位之后一条都不能多。
        let agent = SqliteAuditLedger::open_existing(&AGENT_AUDIT_SLOT.path(request.new_root)?)
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
        owners::run_stage(
            &mut recorder,
            &history,
            &context,
            Stage::Control,
            || owners::verify_control(&context, plan.vault_uuid, expected),
            || owners::control(&context, plan.vault_uuid),
        )?;
    }
    if let Some(expected) = &plan.policy {
        owners::run_stage(
            &mut recorder,
            &history,
            &context,
            Stage::Policy,
            || owners::verify_policy(&context, expected),
            || owners::policy(&context, expected),
        )?;
    }
    if let Some(expected) = &plan.agents {
        let refuse = |_: RestoreRefusal| RestoreRefusal::Owner(CheckpointOwner::AgentStore);
        owners::run_stage(
            &mut recorder,
            &history,
            &context,
            Stage::Agents,
            || owners::verify_agents(&AGENT_SLOT.path(request.new_root)?, expected),
            || {
                let facts = owners::agents(
                    &context,
                    &AGENT_SLOT.path(request.new_root).map_err(refuse)?,
                )?;
                AGENT_SLOT.publish(request.new_root).map_err(refuse)?;
                Ok(facts)
            },
        )?;
    }
    if plan.bridge && !history.recorded(RecoveryStep::SyncLeftClosed) {
        // Bridge 绑着旧根的物理身份，这一版不恢复它的运行状态，目录也不放；同步按引导流程从零重新打开。
        recorder.record(
            RecoveryStep::SyncLeftClosed,
            serde_json::json!({ "restore": request.restore_id }),
        )?;
    }
    let refuse_auth = |_: RestoreRefusal| RestoreRefusal::Auth;
    let probe = (request.sessions.clone(), Arc::clone(&ledger));
    let invalidation = owners::run_stage(
        &mut recorder,
        &history,
        &context,
        Stage::Auth,
        || {
            let (sessions, ledger) = probe;
            owners::verify_auth(
                &AUTH_SLOT.path(request.new_root).map_err(refuse_auth)?,
                sessions,
                request.restore_id,
                ledger,
            )
        },
        || {
            let facts = owners::auth(
                &AUTH_SLOT.path(request.new_root).map_err(refuse_auth)?,
                request.sessions,
                request.totp_seed,
                request.restore_id,
                ledger,
            )?;
            // 撤权做完、Auth 句柄已经关了，才改成正式名：之前按正式路径谁都打不开。
            AUTH_SLOT.publish(request.new_root).map_err(refuse_auth)?;
            Ok(facts)
        },
    )?;
    Ok(RestoreReport {
        lineage,
        reused_placement: !placed.is_empty(),
        restored_owners: plan.restored_owners(),
        sync_left_closed: plan.bridge,
        invalidation,
    })
}

/// 恢复记录里有没有这次恢复（这个恢复标识）的放置意图。有：新根可能已经写过，没做完只能接着这份记录接手；
/// 没有：新根上一个字节都没写过（放置意图交出去之后才放第一个组件）。命令行据此告诉人下一步。
#[must_use]
pub fn restore_started(transcript: &RecoveryTranscript, restore_id: &str) -> bool {
    History::of(transcript, restore_id).started
}

/// 组件在新根里的状态：（已放好——正式名或暂用名都算，上一次放置中途失败留下的 `.restoring` 还在）。放在别的
/// 组件里面的，外层还没放好就当两样都没有。
fn state(slot: &Slot, new_root: &Path) -> Result<(bool, bool), RestoreRefusal> {
    if let Some(parent) = slot.parent
        && !state(parent, new_root)?.0
    {
        return Ok((false, false));
    }
    let parent = slot.parent_path(new_root)?;
    let mut present = false;
    let mut interrupted = false;
    for name in [Some(slot.name), slot.held].into_iter().flatten() {
        present |= RestoredCheckpoint::placed(&parent, name).map_err(placement)?;
        interrupted |= RestoredCheckpoint::interrupted(&parent, name).map_err(placement)?;
    }
    Ok((present, interrupted))
}

/// 读写者数据根上的 lineage，按这次恢复判断（[`classify`]）。
fn found(
    request: &NewRootRestore<'_>,
    plan: &Plan,
    data_root: &Path,
) -> Result<Found, RestoreRefusal> {
    classify(
        read_restore_lineage(data_root).map_err(|_| RestoreRefusal::Rebind)?,
        request.restored.reference().checkpoint_id.as_str(),
        request.restore_id,
        plan.vault_uuid,
        plan.pair_id,
    )
}

/// Broker 重绑定。这次的重绑定已经做完的（接手上一次），只认同一次恢复的 lineage，不再重绑一遍；被封存的根
/// 自己的 lineage 只当上一环（[`classify`]）。恢复标识以前用过的，由 Broker 按 lineage 记的那一串拒绝。
fn rebind(
    request: &NewRootRestore<'_>,
    plan: &Plan,
    data_root: &Path,
) -> Result<RestoreLineage, RestoreRefusal> {
    let parent = match found(request, plan, data_root)? {
        Found::Done(lineage) if request.resume_interrupted => return Ok(*lineage),
        Found::Interrupted { parent } if request.resume_interrupted => parent,
        Found::Done(_) | Found::Interrupted { .. } => return Err(RestoreRefusal::Rebind),
        Found::Fresh { parent } => parent,
    };
    let receipt = rebind_new_roots(&NewRootRebind {
        new_vault_root: &request.new_root.join("vault"),
        new_data_root: data_root,
        expected_vault_uuid: plan.vault_uuid,
        expected_pair_id: plan.pair_id,
        checkpoint_id: request
            .restored
            .reference()
            .checkpoint_id
            .as_str()
            .to_owned(),
        restore_id: request.restore_id,
        parent_lineage: parent,
        resume_interrupted: request.resume_interrupted,
    })
    .map_err(|error| match error {
        RestoreError::RestoreIdReused => RestoreRefusal::RestoreIdUsed,
        _ => RestoreRefusal::Rebind,
    })?;
    if receipt.vault_uuid != plan.vault_uuid || receipt.pair_id != plan.pair_id {
        return Err(RestoreRefusal::Rebind);
    }
    Ok(receipt.lineage)
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
/// 黑板三件的失效也都做完了。恢复标识不会撞上这份数据以前的：Broker 重绑定按 lineage 记的那一串拒绝复用。
///
/// 不带写者数据根启动（只读模式）时没有 lineage 可查，这道闸门管不到；那条路由暂用名管：Auth 撤权做完之前它在
/// `auth.held` 下，按正式路径打不开（身份库同理，agent-server 按正式路径打不开 `agent.held`）。有人特意把配置
/// 指到暂用名，两道都绕得过去，所以恢复出来的新根照旧要按生产配置带写者数据根启动。
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
