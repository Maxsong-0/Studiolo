//! 六个实际 owner 和独立双 Audit；Bridge 未装配明确 absent，不用模拟 Gate 假称七个全装。
use super::{
    checkpoint::{flush_content, source, start, target},
    fixture::Fixture,
};
use crate::{
    PolicyPlane,
    board::BoardPlane,
    checkpoint::{CheckpointInputs, CheckpointOwners, OwnerExportPaths},
};
use std::{
    fs,
    num::NonZeroU64,
    os::unix::fs::PermissionsExt as _,
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_auth::{OsSessionRandom, SessionClock, SessionManager, SystemSessionClock};
use studiolo_backup::worker::{
    job::backup_intent, recovery_point::BackupTargetKind, state::JobKey,
};
use studiolo_core::checkpoint::{
    CheckpointId, CheckpointOwner as Owner, FreezeBudget, FreezeRequest, OwnerAssembly,
    OwnerWatermark,
};
use studiolo_host_config::Config;
use studiolo_policy::PolicyStore;
use studiolo_supply::AgentStore;

mod drill;
mod fixture;
mod producer;
mod producer_auth_backlog;
mod producer_catch_up;
mod producer_check;
mod producer_window;
mod restore;
mod restore_plan;
mod seal;
use fixture::Setup;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_actual_owners_export_the_frozen_moment_with_two_cutoffs_after_every_barrier_is_released()
 {
    let f = Setup::new(true).await;
    let paths = f.targets();
    // 直接追加、没有持久 ACK 的事件（有动作的租约对账、黑板操作的意图都是这样落账的）。以前空的启动对账
    // 每次都会落一对；T-213 起没事可做的对账不再落账，这里手工追加一条代替，保住下面「完整验链覆盖它、
    // 但不虚增 ACK 计数」的检查。
    f.human
        .append(studiolo_audit_ledger::AuditEvent::new(
            studiolo_audit_ledger::AuditEventId::parse("board:task.reconcile:direct").unwrap(),
            studiolo_audit_ledger::AuditPhase::Intent,
            "task.reconcile",
            studiolo_audit_ledger::SanitizedPayload::try_from(
                serde_json::json!({"request_hash": "0".repeat(64)}),
            )
            .unwrap(),
        ))
        .unwrap();
    // 截止点取冻结前原链，不能只数带 ACK 的三条。
    let records = f
        .human
        .query(&studiolo_audit_ledger::AuditFilter::default())
        .unwrap();
    let expected_main = records.iter().map(|record| record.offset()).max().unwrap();
    let frozen = f
        .owners
        .freeze(
            f.request(30_000),
            &f.key,
            BackupTargetKind::LocalTemporaryRepository,
            paths,
        )
        .unwrap();
    f.writable(false);
    assert!(f.workbench.core.owner.lock().unwrap().is_none());
    // 放开（BUG-045）：各库写锁、写者、两边的完整操作准入全部还回去，导出从定格做。
    let mut released = frozen
        .release(Instant::now() + Duration::from_secs(60))
        .unwrap();
    f.writable(true);
    assert!(f.human.enter_operation().is_ok());
    assert!(f.agent.enter_operation().is_ok());
    assert!(f.workbench.core.owner.lock().unwrap().is_some());
    // 放开之后追加的不进这次封存：截止点仍是冻结那一刻。
    f.human
        .append(studiolo_audit_ledger::AuditEvent::new(
            studiolo_audit_ledger::AuditEventId::parse("board:task.reconcile:after").unwrap(),
            studiolo_audit_ledger::AuditPhase::Intent,
            "task.reconcile",
            studiolo_audit_ledger::SanitizedPayload::try_from(
                serde_json::json!({"request_hash": "1".repeat(64)}),
            )
            .unwrap(),
        ))
        .unwrap();
    let exports = released.export().unwrap();
    assert_eq!(exports.owners.len(), 6);
    assert_eq!(
        exports.roster.assembly(Owner::Bridge),
        OwnerAssembly::Absent
    );
    for owner in [
        Owner::Broker,
        Owner::Control,
        Owner::Auth,
        Owner::AgentStore,
        Owner::Policy,
        Owner::Audit,
    ] {
        assert_eq!(exports.roster.assembly(owner), OwnerAssembly::Present);
    }
    let audit = exports
        .owners
        .iter()
        .find(|o| o.owner == Owner::Audit)
        .unwrap();
    let OwnerWatermark::Audit {
        last_offset,
        agent_ledger: Some(agent),
        ..
    } = &audit.watermark
    else {
        panic!("双截止点缺失");
    };
    assert_eq!(*last_offset, expected_main);
    assert_eq!(agent.last_offset, 0);
    assert!(exports.audit_coverage.is_exact());
    // 启动对账直接追加的事件没有持久 ACK；完整验链覆盖它们，但不能虚增 ACK 计数。
    assert!(expected_main > 3);
    assert_eq!(exports.audit_coverage.acked_events_checked, 3);
    assert!(released.export().is_err());
    drop(released);
    f.writable(true);
    f.workbench.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_missing_persisted_intent_unwinds_all_actual_owners_and_both_operation_gates() {
    let f = Setup::new(false).await;
    assert!(
        f.owners
            .freeze(
                f.request(30_000),
                &f.key,
                BackupTargetKind::LocalTemporaryRepository,
                f.targets()
            )
            .is_err()
    );
    f.writable(true);
    assert!(f.human.enter_operation().is_ok());
    assert!(f.agent.enter_operation().is_ok());
    assert!(f.workbench.core.owner.lock().unwrap().is_some());
    f.workbench.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_waits_for_agent_operation_and_never_refreshes_expired_global_budget() {
    let f = Setup::new(true).await;
    let active = f.agent.enter_operation().unwrap();
    let started = Instant::now();
    assert!(
        f.owners
            .freeze(
                f.request(40),
                &f.key,
                BackupTargetKind::LocalTemporaryRepository,
                f.targets()
            )
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(f.workbench.core.owner.lock().unwrap().is_some());
    drop(active);
    f.writable(true);
    f.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_minimal_actual_assembly_does_not_invent_optional_stores_or_second_audit() {
    let f = Fixture::new();
    let (mut broker, audit) = source(&f);
    flush_content(&mut broker, &audit, 0);
    let workbench = start(&f, broker, audit.clone()).await;
    let config = Config::load(&f.config).unwrap();
    let sessions = Arc::new(
        SessionManager::open(
            &f.root.join("auth"),
            config.sessions().unwrap(),
            audit.clone(),
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    let owners = CheckpointOwners::new(CheckpointInputs {
        workbench: workbench.clone(),
        sessions,
        factors: None,
        audit: audit.clone(),
        board: None,
        bridge: None,
        policy: None,
    })
    .unwrap();
    let key = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("minimal-actual").unwrap(),
    };
    audit
        .append(backup_intent(&key, BackupTargetKind::LocalTemporaryRepository).unwrap())
        .unwrap();
    let paths = OwnerExportPaths::under(&target(&f, "exports"));
    for path in paths.directories() {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let frozen = owners
        .freeze(
            FreezeRequest {
                checkpoint_id: key.checkpoint_id.clone(),
                budget: FreezeBudget::new(30_000).unwrap(),
            },
            &key,
            BackupTargetKind::LocalTemporaryRepository,
            paths,
        )
        .unwrap();
    let mut released = frozen
        .release(Instant::now() + Duration::from_secs(60))
        .unwrap();
    let exports = released.export().unwrap();
    assert_eq!(exports.owners.len(), 3);
    for owner in [
        Owner::Control,
        Owner::Bridge,
        Owner::AgentStore,
        Owner::Policy,
    ] {
        assert_eq!(exports.roster.assembly(owner), OwnerAssembly::Absent);
    }
    assert!(matches!(
        &exports.owners.last().unwrap().watermark,
        OwnerWatermark::Audit {
            agent_ledger: None,
            ..
        }
    ));
    drop(released);
    workbench.shutdown().await.unwrap();
}
