//! 新根恢复的编排：按已核验清单里的名单，逐个走每个 owner 自己的恢复入口。
//!
//! 这一版只有三个 owner 有定义好的新根恢复协议：Broker（专用重绑定）、Audit（只开已有库并核对
//! 链尾）、Auth（现成的离线恢复工具 `studiolo-auth-admin`，要人从标准输入给 TOTP 种子）。名单里别的
//! owner 在场（Control、Bridge、AgentStore、Policy）时，整次恢复在写任何东西之前拒绝：它们恢复之后
//! 必须做的事（旧 token 和实例失效、吊销核对、同步 binding 重建）还没有协议，硬开等于让旧凭据和
//! 已吊销的实例复活。
//!
//! 这里做到「四个组件已放到新根、Broker 已重绑定、人侧账本链尾和清单对上」为止。Auth 的库只放置、
//! 不打开：废止旧会话、L3 和恢复码由离线恢复工具在同一个新根上做（它要的布局正是这里的 `auth` 和
//! `audit` 两个名字）。这里不建空库、不补默认 owner、不做最终切换。
use super::export::{AUDIT, AUTH, CANONICAL, WRITER_DATA};
use std::{path::Path, time::Instant};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_backup::{
    checkpoint::staging::{Placement, RestoredCheckpoint},
    recovery::{
        kit::{PreRestoreChecks, RecoveryGrant},
        transcript::RecoveryStep,
    },
};
use studiolo_core::{
    VaultUuid,
    checkpoint::{CheckpointOwner, OwnerAssembly, OwnerWatermark},
};
use studiolo_vault_broker::{NewRootRebind, RestoreReceipt, rebind_new_roots};

/// 封存里的组件目录 → 新根下面的固定名字；名字不由调用方给。
const LAYOUT: [(&str, &str); 4] = [
    (CANONICAL, "vault"),
    (WRITER_DATA, "data"),
    (AUDIT, "audit"),
    (AUTH, "auth"),
];

/// 一次新根恢复的输入。
pub struct NewRootRestore<'a> {
    /// 已经只读核验过的取回副本。
    pub restored: &'a RestoredCheckpoint,
    /// 新根：已经存在、属于当前账号、权限 0700 的目录；四个组件放在它下面的固定名字里。
    pub new_root: &'a Path,
    /// 本次恢复的标识，只写进恢复记录；接下来给 Auth 离线恢复工具的应当是同一个。
    pub restore_id: &'a str,
    /// 上一次恢复的 lineage 标识；第一次恢复为 `None`。
    pub parent_lineage: Option<String>,
    /// 明确接手上一次中途失败留下的新根。为 false 时新根里已有任何一个固定名字都拒绝。
    pub resume_interrupted: bool,
    /// 恢复写入之前人已经确认过的前置检查。
    pub checks: PreRestoreChecks,
    /// 整次恢复共用的截止时间。
    pub deadline: Instant,
}

/// 固定的拒绝类别；不带路径、键和底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreRefusal {
    /// 恢复记录不是这个 checkpoint 的，或者还没取得写入授权。
    Transcript,
    /// 前置检查没有全部确认。
    ChecksIncomplete,
    /// 名单里这个 owner 在场，而它没有新根恢复协议。写任何东西之前就拒绝。
    Unsupported(CheckpointOwner),
    /// 名单或水位不成套：必有的 owner 缺席，或者水位缺项。
    Roster,
    /// 新根里已经有固定名字，而调用方没有声明接手上一次的恢复。
    AlreadyPlaced,
    /// 封存内容没能逐字节放到新根。
    Placement,
    /// Broker 的新根重绑定没有完成。
    Rebind,
    /// 人侧账本打不开，或者链尾和清单里的水位对不上。
    Audit,
}
impl std::fmt::Display for RestoreRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "新根恢复未完成：{self:?}")
    }
}
impl std::error::Error for RestoreRefusal {}

/// 做到「可以交给 Auth 离线恢复工具」时的回执；不是服务可以上线的证明。
#[derive(Debug)]
pub struct RestoreReport {
    /// Broker 重绑定的回执（保留下来的 Vault 身份、配对标识和 lineage）。
    pub receipt: RestoreReceipt,
    /// 这次有没有跳过放置：接手上一次时已经在新根里的组件不重写，也不重新核对内容。
    pub reused_placement: bool,
}

/// 清单里读出来、恢复要对着核的事实。
struct Plan {
    vault_uuid: VaultUuid,
    pair_id: VaultUuid,
    audit_offset: u64,
    audit_hash: String,
}

/// 把一份已核验的取回副本恢复到新根。
///
/// 顺序：核恢复记录和前置检查 → 按名单定能不能恢复（不能就在这里拒绝，还没写任何东西）→ 逐字节
/// 核对着放置四个组件 → Broker 重绑定 → 重开人侧账本并核链尾。每一步都记进恢复记录。
/// 成功返回后还剩两步，都不在这里做：Auth 离线恢复工具废止旧凭据，然后人做最终切换。
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
    let mut present = 0;
    for (_, name) in LAYOUT {
        if RestoredCheckpoint::placed(request.new_root, name)
            .map_err(|_| RestoreRefusal::Placement)?
        {
            present += 1;
        }
    }
    if present > 0 && !request.resume_interrupted {
        return Err(RestoreRefusal::AlreadyPlaced);
    }
    record(
        grant,
        RecoveryStep::PreRestoreChecksCompleted,
        serde_json::json!({ "checks": request.checks }),
    )?;
    record(
        grant,
        RecoveryStep::PhysicalRebindIntent,
        serde_json::json!({ "restore": request.restore_id, "reused_components": present }),
    )?;

    for (component, name) in LAYOUT {
        let placement = request
            .restored
            .extract(component, request.new_root, name, request.deadline)
            .map_err(|_| RestoreRefusal::Placement)?;
        if placement == Placement::AlreadyPresent && !request.resume_interrupted {
            return Err(RestoreRefusal::AlreadyPlaced);
        }
    }

    let receipt = rebind_new_roots(&NewRootRebind {
        new_vault_root: &request.new_root.join("vault"),
        new_data_root: &request.new_root.join("data"),
        expected_vault_uuid: plan.vault_uuid,
        expected_pair_id: plan.pair_id,
        checkpoint_id: checkpoint.as_str().to_owned(),
        parent_lineage: request.parent_lineage,
        resume_interrupted: request.resume_interrupted,
    })
    .map_err(|_| RestoreRefusal::Rebind)?;
    if receipt.vault_uuid != plan.vault_uuid || receipt.pair_id != plan.pair_id {
        return Err(RestoreRefusal::Rebind);
    }
    record(
        grant,
        RecoveryStep::PhysicalRebindOutcome,
        serde_json::json!({ "restore": request.restore_id }),
    )?;

    let ledger = SqliteAuditLedger::open_existing(&request.new_root.join("audit"))
        .map_err(|_| RestoreRefusal::Audit)?;
    // 第一次恢复时链尾必须正好是清单里的水位；接手上一次时后面可以多出恢复自己记的事件，
    // 但水位那一条必须原样还在。
    let tail = ledger
        .query(
            &AuditFilter::default()
                .after_offset(plan.audit_offset - 1)
                .with_limit(2),
        )
        .map_err(|_| RestoreRefusal::Audit)?;
    let anchored = tail.first().is_some_and(|entry| {
        entry.offset() == plan.audit_offset && entry.record_hash().to_hex() == plan.audit_hash
    });
    if !anchored || (tail.len() != 1 && !request.resume_interrupted) {
        return Err(RestoreRefusal::Audit);
    }
    Ok(RestoreReport {
        receipt,
        reused_placement: present > 0,
    })
}

/// 只看已核验的清单，定这份封存这一版能不能恢复；不碰磁盘。
fn plan(restored: &RestoredCheckpoint) -> Result<Plan, RestoreRefusal> {
    let manifest = restored.manifest();
    for (who, assembly) in manifest.roster.entries() {
        let restorable = matches!(
            who,
            CheckpointOwner::Broker | CheckpointOwner::Auth | CheckpointOwner::Audit
        );
        match (assembly, restorable) {
            (OwnerAssembly::Present, false) => return Err(RestoreRefusal::Unsupported(who)),
            (OwnerAssembly::Absent, true) => return Err(RestoreRefusal::Roster),
            _ => {}
        }
    }
    let mut broker = None;
    let mut audit = None;
    let mut auth = false;
    for watermark in &manifest.watermarks {
        match watermark {
            OwnerWatermark::Broker {
                vault_uuid,
                pair_id,
                unresolved: false,
                ..
            } => broker = Some((*vault_uuid, *pair_id)),
            // 带第二条账本的封存一定同时有 AgentStore；名单那一关已经拒绝过，这里再守一次。
            OwnerWatermark::Audit {
                last_offset,
                last_hash,
                agent_ledger: None,
            } if *last_offset > 0 => audit = Some((*last_offset, last_hash.clone())),
            OwnerWatermark::Auth { .. } => auth = true,
            _ => return Err(RestoreRefusal::Roster),
        }
    }
    let ((vault_uuid, pair_id), (audit_offset, audit_hash)) = broker
        .zip(audit)
        .filter(|_| auth)
        .ok_or(RestoreRefusal::Roster)?;
    Ok(Plan {
        vault_uuid,
        pair_id,
        audit_offset,
        audit_hash,
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
