//! 组件放好、Broker 重绑定之后，每个 owner 要做的失效（T-212 设计 4.1～4.4）和账本链尾核对。
//!
//! 每个 owner 的步骤都一个形状（[`run_stage`]）：这次恢复里还没开始过的，先在恢复记录里记「开始」，按清单里
//! 它的水位重算核对，再做失效、做事后核对、记「完成」；已经开始过的（接手上一次），它的库已经改过，不再按封存
//! 水位核，只做幂等的失效和事后核对。失效本身都是幂等的：同一恢复标识再做一遍什么都不变。
use super::{RestoreRefusal, record};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path, sync::Arc, time::Instant};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};
use studiolo_auth::{
    OsSessionRandom, Recovery, RestoreInvalidation, SessionConfig, SessionManager,
    SystemSessionClock, Totp, TotpSeed,
};
use studiolo_backup::recovery::{
    kit::RecoveryGrant,
    transcript::{RecoveryStep, RecoveryTranscript},
};
use studiolo_control_db::ControlDb;
use studiolo_core::{
    VaultUuid,
    checkpoint::{AuditChainCutoff, CheckpointOwner, OwnerWatermark},
};
use studiolo_policy::{PolicyStore, RevocationLevel};
use studiolo_supply::invalidate_restored_identities;

/// 恢复写进策略库的吊销者名字。
const ACTOR: &str = "restore";

/// 要做失效的 owner；Auth 撤权也按同一个形状走，而且永远最后做。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Control,
    Policy,
    Agents,
    Auth,
}
impl Stage {
    /// 写进「开始」记录的 owner 名字。
    fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Policy => "policy",
            Self::Agents => "agent_store",
            Self::Auth => "auth",
        }
    }
    /// 这个 owner 做完时记的步骤。
    fn outcome(self) -> RecoveryStep {
        match self {
            Self::Control => RecoveryStep::ControlLeasesRevoked,
            Self::Policy => RecoveryStep::PolicyGloballyRevoked,
            Self::Agents => RecoveryStep::AgentCredentialsInvalidated,
            Self::Auth => RecoveryStep::AuthInvalidated,
        }
    }
}

/// 同一份恢复记录里，这次恢复（同一恢复标识）之前已经走到哪了。
pub(super) struct History {
    /// 记录里有这次恢复的放置意图：接手的前提。
    pub started: bool,
    /// 记录里有任何一次恢复的放置意图：新开一次恢复的记录必须没有。
    pub any_started: bool,
    begun: BTreeSet<String>,
    recorded: Vec<RecoveryStep>,
}
impl History {
    pub fn of(transcript: &RecoveryTranscript, restore_id: &str) -> Self {
        let mut history = Self {
            started: false,
            any_started: false,
            begun: BTreeSet::new(),
            recorded: Vec::new(),
        };
        for entry in &transcript.entries {
            let payload = entry.payload.as_value();
            if entry.step == RecoveryStep::PhysicalRebindIntent {
                history.any_started = true;
            }
            if payload.get("restore").and_then(Value::as_str) != Some(restore_id) {
                continue;
            }
            match entry.step {
                RecoveryStep::PhysicalRebindIntent => history.started = true,
                RecoveryStep::OwnerRestoreIntent => {
                    if let Some(owner) = payload.get("owner").and_then(Value::as_str) {
                        history.begun.insert(owner.to_owned());
                    }
                }
                step => history.recorded.push(step),
            }
        }
        history
    }
    /// 这个 owner 的失效在这次恢复里开始过：它的库可能已经改过。
    pub fn begun(&self, stage: Stage) -> bool {
        self.begun.contains(stage.name()) || self.recorded(stage.outcome())
    }
    /// 这次恢复已经记过这一步。
    pub fn recorded(&self, step: RecoveryStep) -> bool {
        self.recorded.contains(&step)
    }
}

/// 各步骤共用的输入。
pub(super) struct Context<'a> {
    pub new_root: &'a Path,
    pub restore_id: &'a str,
    pub now: &'a jiff::Timestamp,
    /// 写进策略库吊销记录的理由（带恢复标识）。
    pub reason: &'a str,
    pub deadline: Instant,
}

/// 按固定形状走一个 owner 的失效：没开始过的先记「开始」，`work(true)` 先核清单水位再动手；开始过的
/// `work(false)` 只做幂等的失效和事后核对。「完成」只记一次，载荷是 `work` 交回的事实（不带凭据）。
pub(super) fn run_stage<T>(
    grant: &mut RecoveryGrant,
    history: &History,
    context: &Context<'_>,
    stage: Stage,
    work: impl FnOnce(bool) -> Result<(T, Value), RestoreRefusal>,
) -> Result<T, RestoreRefusal> {
    let begun = history.begun(stage);
    if !begun {
        record(
            grant,
            RecoveryStep::OwnerRestoreIntent,
            json!({ "owner": stage.name(), "restore": context.restore_id }),
        )?;
    }
    let (value, mut facts) = work(!begun)?;
    if !history.recorded(stage.outcome()) {
        let Some(object) = facts.as_object_mut() else {
            return Err(RestoreRefusal::Transcript);
        };
        object.insert("restore".into(), context.restore_id.into());
        record(grant, stage.outcome(), facts)?;
    }
    Ok(value)
}

fn refuse<E>(owner: CheckpointOwner) -> impl FnOnce(E) -> RestoreRefusal {
    move |_| RestoreRefusal::Owner(owner)
}

/// Control（设计 4.2）：只打开放出来的库（不建、不迁移），第一次先核水位，再按启动巡检撤销全部活动 attempt。
/// 事后核对：没有 attempt 还拿着租约（已认领、运行中）；只在等子任务的转成取消中，工具能力已经收回。
/// 过期提交按 2026-11-16 拍板停在待审，恢复不替主人丢。
pub(super) fn control(
    context: &Context<'_>,
    vault: VaultUuid,
    expected: &OwnerWatermark,
    fresh: bool,
) -> Result<((), Value), RestoreRefusal> {
    let owner = CheckpointOwner::Control;
    let mut db = ControlDb::open_existing(&context.new_root.join("control"), vault)
        .map_err(refuse(owner))?;
    if fresh && db.restored_watermark().map_err(refuse(owner))? != *expected {
        return Err(RestoreRefusal::Owner(owner));
    }
    let reconciled = db
        .reconcile_leases(context.now.as_millisecond(), true)
        .map_err(refuse(owner))?;
    let live = db.live_attempts().map_err(refuse(owner))?;
    if live.leased != 0 {
        return Err(RestoreRefusal::Owner(owner));
    }
    Ok((
        (),
        json!({
            "abandoned": reconciled.abandoned.len(),
            "cancel_requested": live.cancel_requested,
        }),
    ))
}

/// Policy（设计 4.4，用户 2026-11-16 拍板「一律全局吊销」）：第一次先核水位，再全局吊销，只能由主人经 L3 解除。
/// 封存截止点时已经是全局吊销的，原记录保留、不再新加一条。
pub(super) fn policy(
    context: &Context<'_>,
    expected: &OwnerWatermark,
    fresh: bool,
) -> Result<((), Value), RestoreRefusal> {
    let owner = CheckpointOwner::Policy;
    let OwnerWatermark::Policy { global_revoked, .. } = expected else {
        return Err(RestoreRefusal::Roster);
    };
    let store =
        PolicyStore::open_existing(&context.new_root.join("policy")).map_err(refuse(owner))?;
    if fresh
        && store
            .restored_watermark(context.deadline)
            .map_err(refuse(owner))?
            != *expected
    {
        return Err(RestoreRefusal::Owner(owner));
    }
    let newly = store
        .revoke(
            RevocationLevel::Global,
            "*",
            ACTOR,
            context.reason,
            &context.now.to_string(),
        )
        .map_err(refuse(owner))?;
    if !store.effective().map_err(refuse(owner))?.global_revoked {
        return Err(RestoreRefusal::Owner(owner));
    }
    Ok((
        (),
        json!({ "newly_revoked": newly, "revoked_at_cutoff": global_revoked }),
    ))
}

/// AgentStore（设计 4.3）：第一次先核水位，吊销全部类型（旧 token 一律失效），每个已登记实例在策略库记实例
/// 吊销（不可恢复）。之后由主人重新签发类型 token、AI 重新登记。事后核对：每个实例在策略库都是吊销状态。
pub(super) fn agents(
    context: &Context<'_>,
    expected: &OwnerWatermark,
    fresh: bool,
) -> Result<((), Value), RestoreRefusal> {
    let owner = CheckpointOwner::AgentStore;
    let now = context.now.to_string();
    let identities = invalidate_restored_identities(
        &context.new_root.join("agent"),
        fresh.then_some(expected),
        &now,
    )
    .map_err(refuse(owner))?;
    let store =
        PolicyStore::open_existing(&context.new_root.join("policy")).map_err(refuse(owner))?;
    for instance in &identities.instances {
        store
            .revoke(
                RevocationLevel::Instance,
                instance,
                ACTOR,
                context.reason,
                &now,
            )
            .map_err(refuse(owner))?;
    }
    let revoked: BTreeSet<String> = store
        .revocations(true)
        .map_err(refuse(owner))?
        .into_iter()
        .filter(|record| record.level == RevocationLevel::Instance)
        .map(|record| record.target)
        .collect();
    if identities
        .instances
        .iter()
        .any(|instance| !revoked.contains(instance))
    {
        return Err(RestoreRefusal::Owner(owner));
    }
    Ok((
        (),
        json!({
            "types_revoked": identities.types_revoked,
            "types": identities.types_total,
            "instances": identities.instances.len(),
        }),
    ))
}

/// Auth 撤权（设计 4.1），照 `studiolo-auth-admin restore` 的顺序：只为恢复打开已有库 → 挂 TOTP 和恢复码因子 →
/// 废止全部旧会话、L3 和恢复码。要求身份版本变大（接手上一次时等于当时提交的那次、之后没再变）、
/// 两条恢复事件都投进了新根的人侧账本；否则拒绝，启动闸门照旧不放行。
///
/// 这里不调 `AuthRestoreInvalidation::check`：它要的「主人已重新认证」只有最终切换之后才成立，归切换那一步核。
pub(super) fn auth(
    new_root: &Path,
    sessions: SessionConfig,
    seed: TotpSeed,
    restore_id: &str,
    ledger: Arc<SqliteAuditLedger>,
) -> Result<(RestoreInvalidation, Value), RestoreRefusal> {
    let manager = Arc::new(
        SessionManager::open_existing_for_restore(
            &new_root.join("auth"),
            sessions,
            ledger,
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )
        .map_err(|_| RestoreRefusal::Auth)?,
    );
    let earlier = manager
        .restored_epoch(restore_id)
        .map_err(|_| RestoreRefusal::Auth)?;
    let before = manager.identity_epoch().map_err(|_| RestoreRefusal::Auth)?;
    let (_totp, _enrollment) =
        Totp::attach(manager.clone(), seed).map_err(|_| RestoreRefusal::Auth)?;
    let _recovery = Recovery::attach(manager.clone()).map_err(|_| RestoreRefusal::Auth)?;
    let report = manager
        .invalidate_restored_credentials(restore_id)
        .map_err(|_| RestoreRefusal::Auth)?;
    let advanced = match earlier {
        None => report.epoch > before,
        Some(epoch) => report.epoch == epoch && epoch == before,
    };
    let delivered = manager
        .restore_completed(restore_id)
        .map_err(|_| RestoreRefusal::Auth)?;
    if !(advanced && report.audit_confirmed && delivered) {
        return Err(RestoreRefusal::Auth);
    }
    let facts = json!({
        "epoch": report.epoch,
        "revoked_sessions": report.revoked_sessions,
        "cleared_l3": report.cleared_l3,
        // 字段名照 Auth 恢复事件的写法：末两词是 recovery codes 的会被当成凭据拒掉。
        "invalidated_recovery_code_count": report.invalidated_recovery_codes,
    });
    Ok((report, facts))
}

/// 账本链尾核对：清单里水位那一条必须原样还在（水位是 0 的，封存时账本是空的）；之后多出的条目只允许是这次
/// 恢复 Auth 撤权自己记的（`auth_restore` 给了恢复标识时，最多两条），别的一律拒绝。
pub(super) fn check_tail(
    ledger: &SqliteAuditLedger,
    cutoff: &AuditChainCutoff,
    auth_restore: Option<&str>,
) -> Result<(), ()> {
    let filter = AuditFilter::default().with_limit(4);
    let filter = if cutoff.last_offset == 0 {
        filter
    } else {
        filter.after_offset(cutoff.last_offset - 1)
    };
    let entries = ledger.query(&filter).map_err(drop)?;
    let extras = if cutoff.last_offset == 0 {
        &entries[..]
    } else {
        let (anchor, rest) = entries.split_first().ok_or(())?;
        if anchor.offset() != cutoff.last_offset
            || anchor.record_hash().to_hex() != cutoff.last_hash
        {
            return Err(());
        }
        rest
    };
    let ours = |entry: &AuditRecord| {
        let event = entry.event();
        let payload = event.payload().as_value();
        auth_restore.is_some_and(|restore| {
            event.action() == "session.revoked"
                && payload["scope"] == "restore"
                && payload["target"] == restore
        })
    };
    if extras.len() > 2 || !extras.iter().all(ours) {
        return Err(());
    }
    Ok(())
}
