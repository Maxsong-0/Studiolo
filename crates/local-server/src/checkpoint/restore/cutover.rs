//! 最终切换（T-212 设计 4.6）：恢复做完、人核对过之后，由人分两步执行。这里不改配置、不起停服务，只核前提，
//! 再把这一步记进恢复记录（和恢复一样，每条先交给去处）。
//!
//! - 开始（[`begin_cutover`]）：这次恢复的 Auth 撤权已经记在恢复记录里、新根的 lineage 是这次恢复的、启动闸门
//!   放行 → 记 `FinalCutoverIntent`。之后由人把生产配置指到新根、起服务、重新登录、核对内容。
//! - 完成（[`finish_cutover`]）：记过开始、主人在新根上重新登录过、失效事实按
//!   `AuthRestoreInvalidation::check` 全部成立 → 记 `FinalCutoverOutcome`。这里按离线恢复的方式打开 Auth，
//!   所以先停掉新根上的服务再做。
//!
//! 不打算切换的（回到旧根）不用这里：旧根没动过，新根由人删。
use super::{Journal, Recorder, owners::History, require_restore_revoked};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{OsSessionRandom, SessionConfig, SessionManager, SystemSessionClock};
use studiolo_backup::recovery::{
    kit::{AuthRestoreInvalidation, RecoveryGrant},
    transcript::{RecoveryStep, RecoveryTranscript},
};
use studiolo_core::checkpoint::CheckpointOwner;
use studiolo_vault_broker::read_restore_lineage;

/// 最终切换一步的输入。
pub struct Cutover<'a> {
    /// 恢复出来的新根（恢复时用的那一个）。
    pub new_root: &'a Path,
    /// 这次恢复的标识。
    pub restore_id: &'a str,
    /// 主人的会话配置（来自私有配置），打开新根上的 Auth 用。
    pub sessions: SessionConfig,
}

/// 最终切换的固定拒绝类别；不带路径、键和底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutoverRefusal {
    /// 恢复记录不自洽、取不到写入授权、缺这次恢复该有的事实，或者交不出去。
    Transcript,
    /// 次序不对：这次恢复还没做完（记录里没有它的 Auth 撤权完成）；开始时已经记过开始；完成时还没记开始、
    /// 或者已经完成过。
    Sequence,
    /// 新根对不上：lineage 不是这次恢复的、账本或 Auth 打不开（新根上的服务还在跑也会这样），或者启动闸门
    /// 不放行。
    NewRoot,
    /// 主人还没在新根上重新登录过。
    NotReauthenticated,
    /// 失效事实核对不过（`AuthRestoreInvalidation::check`），或者恢复记录里的身份版本和 Auth 里的对不上。
    Invalidation,
}
impl std::fmt::Display for CutoverRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "最终切换没有记：{self:?}")
    }
}
impl std::error::Error for CutoverRefusal {}

/// 开始最终切换：核过前提之后记 `FinalCutoverIntent`。
pub fn begin_cutover(
    grant: &mut RecoveryGrant,
    cutover: Cutover<'_>,
    journal: &mut Journal<'_>,
) -> Result<(), CutoverRefusal> {
    let history = history(grant, cutover.restore_id)?;
    if !history.recorded(RecoveryStep::AuthInvalidated)
        || history.recorded(RecoveryStep::FinalCutoverIntent)
        || history.recorded(RecoveryStep::FinalCutoverOutcome)
    {
        return Err(CutoverRefusal::Sequence);
    }
    Restored::of(&grant.transcript, cutover.restore_id).ok_or(CutoverRefusal::Transcript)?;
    open(
        grant,
        cutover.new_root,
        cutover.restore_id,
        cutover.sessions,
    )?;
    Recorder { grant, journal }
        .record(
            RecoveryStep::FinalCutoverIntent,
            json!({ "restore": cutover.restore_id }),
        )
        .map_err(|_| CutoverRefusal::Transcript)
}

/// 完成最终切换：主人已经在新根上重新登录过、失效事实全部成立，记 `FinalCutoverOutcome`，交回核过的事实。
pub fn finish_cutover(
    grant: &mut RecoveryGrant,
    cutover: Cutover<'_>,
    journal: &mut Journal<'_>,
) -> Result<AuthRestoreInvalidation, CutoverRefusal> {
    let restore_id = cutover.restore_id;
    let history = history(grant, restore_id)?;
    if !history.recorded(RecoveryStep::FinalCutoverIntent)
        || history.recorded(RecoveryStep::FinalCutoverOutcome)
    {
        return Err(CutoverRefusal::Sequence);
    }
    // 解构着取：守卫把非 owner 文件里「点 owner」的字段访问当越界（`canonical_owner` 用例）。
    let Restored {
        owners: restored,
        epoch,
        previous_epoch,
    } = Restored::of(&grant.transcript, restore_id).ok_or(CutoverRefusal::Transcript)?;
    // 名单里有 Control 的，它的租约必须收回过；别的 owner 由下面的失效事实核。
    if restored.contains(&CheckpointOwner::Control)
        && !history.recorded(RecoveryStep::ControlLeasesRevoked)
    {
        return Err(CutoverRefusal::Invalidation);
    }
    let manager = open(grant, cutover.new_root, restore_id, cutover.sessions)?;
    let unavailable = |_| CutoverRefusal::NewRoot;
    if manager.restored_epoch(restore_id).map_err(unavailable)? != Some(epoch) {
        return Err(CutoverRefusal::Invalidation);
    }
    let reauthenticated = manager
        .reauthenticated_after_restore(restore_id)
        .map_err(unavailable)?;
    if !reauthenticated {
        return Err(CutoverRefusal::NotReauthenticated);
    }
    // 撤权提交了、两条恢复事件都有回执（启动闸门已经核过）：Auth 的撤权在一个事务里同时废止旧会话、L3 和
    // 恢复码，保留已消费记录和审计锚点（`invalidate_restored_credentials` 的约定）。按私有配置里的主人打开
    // 成功，说明主人绑定没变。
    let revoked = manager.restore_completed(restore_id).map_err(unavailable)?;
    let agents = history.recorded(RecoveryStep::AgentCredentialsInvalidated);
    let generation = |epoch: i64| u64::try_from(epoch).map_err(|_| CutoverRefusal::Invalidation);
    let invalidation = AuthRestoreInvalidation {
        old_sessions_invalidated: revoked,
        l3_invalidated: revoked,
        snapshot_recovery_codes_blocked: revoked,
        consumed_records_preserved: revoked,
        audit_anchor_preserved: revoked,
        owner_binding_preserved: true,
        reauthenticated,
        issuance_generation: generation(epoch)?,
        agent_store_included: restored.contains(&CheckpointOwner::AgentStore),
        agent_type_tokens_invalidated: agents,
        agent_instances_invalidated: agents,
        policy_included: restored.contains(&CheckpointOwner::Policy),
        policy_revocations_reconciled: history.recorded(RecoveryStep::PolicyGloballyRevoked),
    };
    invalidation
        .check(generation(previous_epoch)?)
        .map_err(|_| CutoverRefusal::Invalidation)?;
    Recorder { grant, journal }
        .record(
            RecoveryStep::FinalCutoverOutcome,
            json!({
                "restore": restore_id,
                "epoch": epoch,
                "agent_store_included": invalidation.agent_store_included,
                "policy_included": invalidation.policy_included,
            }),
        )
        .map_err(|_| CutoverRefusal::Transcript)?;
    Ok(invalidation)
}

/// 记录要自洽、能取得写入授权，而且是这次恢复的（有它的放置意图）。
fn history(grant: &RecoveryGrant, restore_id: &str) -> Result<History, CutoverRefusal> {
    grant
        .transcript
        .authorize_write()
        .map_err(|_| CutoverRefusal::Transcript)?;
    let history = History::of(&grant.transcript, restore_id);
    if !history.started {
        return Err(CutoverRefusal::Sequence);
    }
    Ok(history)
}

/// 核新根是这次恢复的（Broker 重绑定做完了，恢复标识和 checkpoint 都对得上），按离线恢复的方式打开 Auth，
/// 问启动闸门。
fn open(
    grant: &RecoveryGrant,
    new_root: &Path,
    restore_id: &str,
    sessions: SessionConfig,
) -> Result<SessionManager, CutoverRefusal> {
    let data = new_root.join("data");
    let lineage = read_restore_lineage(&data)
        .map_err(|_| CutoverRefusal::NewRoot)?
        .ok_or(CutoverRefusal::NewRoot)?;
    if !lineage.is_completed()
        || lineage.restore_id.as_deref() != Some(restore_id)
        || lineage.checkpoint_id != grant.transcript.checkpoint_id.as_str()
    {
        return Err(CutoverRefusal::NewRoot);
    }
    let ledger = SqliteAuditLedger::open_existing(&new_root.join("audit"))
        .map_err(|_| CutoverRefusal::NewRoot)?;
    let manager = SessionManager::open_existing_for_restore(
        &new_root.join("auth"),
        sessions,
        Arc::new(ledger),
        Arc::new(SystemSessionClock),
        Arc::new(OsSessionRandom),
    )
    .map_err(|_| CutoverRefusal::NewRoot)?;
    require_restore_revoked(Some(&data), &manager).map_err(|_| CutoverRefusal::NewRoot)?;
    Ok(manager)
}

/// 恢复记录里这次恢复留下的事实：恢复了哪些 owner（放置意图），Auth 撤权前后的身份版本（撤权完成）。
struct Restored {
    owners: Vec<CheckpointOwner>,
    epoch: i64,
    previous_epoch: i64,
}
impl Restored {
    fn of(transcript: &RecoveryTranscript, restore_id: &str) -> Option<Self> {
        let mut owners = None;
        let mut epochs = None;
        for entry in &transcript.entries {
            let payload = entry.payload.as_value();
            if payload.get("restore").and_then(Value::as_str) != Some(restore_id) {
                continue;
            }
            match entry.step {
                RecoveryStep::PhysicalRebindIntent => {
                    owners = Some(serde_json::from_value(payload.get("owners")?.clone()).ok()?);
                }
                RecoveryStep::AuthInvalidated => {
                    epochs = Some((
                        payload.get("epoch")?.as_i64()?,
                        payload.get("previous_epoch")?.as_i64()?,
                    ));
                }
                _ => {}
            }
        }
        let (epoch, previous_epoch) = epochs?;
        Some(Self {
            owners: owners?,
            epoch,
            previous_epoch,
        })
    }
}
