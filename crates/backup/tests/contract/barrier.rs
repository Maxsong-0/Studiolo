//! 行为与失败契约：跨 owner 屏障必须固定顺序、有界等待、逐项 present/absent，
//! 并且拒绝一切「看起来像冻结」的替代物。

use super::support::{
    StubParticipant, TestClock, drained, full_roster, request, transfer_source, verified_checkpoint,
};
use studiolo_backup::checkpoint::barrier::{BarrierError, BrokerPrecondition, CrossOwnerBarrier};
use studiolo_backup::checkpoint::types::SealError;
use studiolo_core::VaultUuid;
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner, CheckpointParticipant, InstantMillis, OwnerAssembly,
    OwnerWatermark,
};
use tempfile::TempDir;

fn settled() -> BrokerPrecondition {
    BrokerPrecondition {
        sole_writer_lease: true,
        unresolved: false,
    }
}

struct World {
    clock: TestClock,
    vault: VaultUuid,
    pair: VaultUuid,
}

impl World {
    fn new() -> Self {
        Self {
            clock: TestClock::new(0),
            vault: VaultUuid::new(),
            pair: VaultUuid::new(),
        }
    }

    fn participants(&self) -> Vec<StubParticipant<'_>> {
        CheckpointOwner::FREEZE_ORDER
            .iter()
            .map(|owner| StubParticipant::new(*owner, &self.clock, self.vault, self.pair))
            .collect()
    }
}

fn refs<'a>(participants: &'a [StubParticipant<'a>]) -> Vec<&'a dyn CheckpointParticipant> {
    participants
        .iter()
        .map(|participant| participant as &dyn CheckpointParticipant)
        .collect()
}

#[test]
fn every_assembled_owner_freezes_in_the_fixed_order_and_the_roster_lists_all_six() {
    let world = World::new();
    let participants = world.participants();
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let observed: Vec<_> = window
        .receipts()
        .iter()
        .map(|receipt| receipt.owner)
        .collect();
    assert_eq!(observed, CheckpointOwner::FREEZE_ORDER.to_vec());
    assert_eq!(
        window.roster().expect("roster").entries().count(),
        6,
        "六个 owner 一个都不能缺项"
    );
    window.thaw();
}

#[test]
fn audit_is_the_last_owner_to_enter_the_storage_freeze() {
    assert_eq!(
        CheckpointOwner::FREEZE_ORDER.last(),
        Some(&CheckpointOwner::Audit),
        "Audit 必须保持可用到其余在途操作实际结束"
    );
}

#[test]
fn holding_only_the_canonical_mutex_is_not_a_cross_owner_freeze() {
    // 只登记 Broker 一个参与者，等价于「拿到 canonical Mutex 就以为冻住了」。
    let world = World::new();
    let participants = vec![StubParticipant::new(
        CheckpointOwner::Broker,
        &world.clock,
        world.vault,
        world.pair,
    )];

    let outcome = CrossOwnerBarrier::assemble(refs(&participants));

    assert!(matches!(
        outcome.err(),
        Some(BarrierError::Owner(CheckpointError::OwnerMissing {
            owner: CheckpointOwner::Control
        }))
    ));
}

#[test]
fn an_out_of_order_or_duplicated_participant_list_is_refused() {
    let world = World::new();
    let mut participants = world.participants();
    participants.swap(0, 1);
    assert!(matches!(
        CrossOwnerBarrier::assemble(refs(&participants)),
        Err(BarrierError::Owner(CheckpointError::OwnerOutOfOrder { .. }))
    ));

    let mut duplicated = world.participants();
    duplicated.push(StubParticipant::new(
        CheckpointOwner::Broker,
        &world.clock,
        world.vault,
        world.pair,
    ));
    assert!(matches!(
        CrossOwnerBarrier::assemble(refs(&duplicated)),
        Err(BarrierError::Owner(CheckpointError::OwnerDuplicated { .. }))
    ));
}

#[test]
fn pausing_http_without_blocking_local_maintenance_writers_is_not_drained() {
    let world = World::new();
    let mut participants = world.participants();
    participants[1].drain.local_maintenance_writers_blocked = false;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    let error = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .err();

    assert!(matches!(
        error,
        Some(BarrierError::Owner(CheckpointError::NotDrained {
            owner: CheckpointOwner::Control
        }))
    ));
}

#[test]
fn auth_implicit_read_writes_must_also_be_blocked() {
    let world = World::new();
    let mut participants = world.participants();
    participants[2].drain.implicit_read_writes_blocked = false;

    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    assert!(matches!(
        barrier.freeze(&request("cp-1", 1_000), settled(), &world.clock),
        Err(BarrierError::Owner(CheckpointError::NotDrained {
            owner: CheckpointOwner::Auth
        }))
    ));
}

#[test]
fn a_readable_prepared_record_does_not_by_itself_block_the_freeze() {
    let world = World::new();
    let mut participants = world.participants();
    participants[0].drain.prepared_readable = true;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("prepared 可读只是事实记录");
    assert!(window.receipts()[0].drain.prepared_readable);
    window.thaw();
}

#[test]
fn one_successful_background_delivery_is_not_all_pending_delivered() {
    let world = World::new();
    let mut participants = world.participants();
    participants[1].drain.background_delivery_pending = 3;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    assert!(matches!(
        barrier.freeze(&request("cp-1", 1_000), settled(), &world.clock),
        Err(BarrierError::Owner(CheckpointError::NotDrained {
            owner: CheckpointOwner::Control
        }))
    ));
}

#[test]
fn waiting_past_the_budget_is_a_timeout_even_when_the_owner_claims_it_drained() {
    let world = World::new();
    let mut participants = world.participants();
    participants[3].freeze_cost = 5_000;
    participants[3].drain = drained();
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    assert!(matches!(
        barrier.freeze(&request("cp-1", 1_000), settled(), &world.clock),
        Err(BarrierError::Owner(CheckpointError::FreezeTimeout {
            owner: CheckpointOwner::Bridge
        }))
    ));
}

#[test]
fn a_failed_freeze_thaws_every_owner_that_was_already_frozen() {
    let world = World::new();
    let mut participants = world.participants();
    participants[3].drain.in_flight_operations = 1;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    assert!(
        barrier
            .freeze(&request("cp-1", 1_000), settled(), &world.clock)
            .is_err()
    );

    for participant in participants.iter().take(4) {
        assert_eq!(
            participant.thawed.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "{:?} 必须被解冻",
            participant.owner
        );
    }
    assert_eq!(
        participants[5]
            .thawed
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "还没冻结的 owner 不该被解冻"
    );
}

#[test]
fn an_unassembled_owner_is_recorded_as_absent_and_is_never_frozen_by_the_backup() {
    let world = World::new();
    let mut participants = world.participants();
    participants[4].assembly = OwnerAssembly::Absent;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let roster = window.roster().expect("roster");
    assert_eq!(
        roster.assembly(CheckpointOwner::AgentStore),
        OwnerAssembly::Absent
    );
    assert_eq!(
        participants[4]
            .thawed
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "备份器不得擅自停止或解冻未装配的独立服务"
    );
    assert_eq!(window.receipts().len(), 5);
    window.thaw();
}

#[test]
fn the_broker_must_hold_the_sole_lease_and_have_no_unresolved_operation() {
    let world = World::new();
    let participants = world.participants();
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    assert!(matches!(
        barrier.freeze(
            &request("cp-1", 1_000),
            BrokerPrecondition {
                sole_writer_lease: false,
                unresolved: false
            },
            &world.clock,
        ),
        Err(BarrierError::BrokerLeaseNotSole)
    ));
    assert!(matches!(
        barrier.freeze(
            &request("cp-1", 1_000),
            BrokerPrecondition {
                sole_writer_lease: true,
                unresolved: true
            },
            &world.clock,
        ),
        Err(BarrierError::BrokerUnresolved)
    ));
}

#[test]
fn an_unverified_owner_export_cannot_become_a_seal_ticket() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let mut participants = world.participants();
    // WAL checkpoint 仍然 busy：复制在线主文件忽略 WAL 的典型形态。
    participants[2].export.wal.busy = 5;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let error = window.seal(
        &request("cp-1", 1_000),
        transfer_source(temporary.path()),
        InstantMillis::from_millis(10),
    );

    assert!(matches!(
        error.err(),
        Some(BarrierError::Owner(CheckpointError::Unverified {
            owner: CheckpointOwner::Auth
        }))
    ));
}

#[test]
fn an_export_that_was_never_reopened_or_synced_is_refused() {
    let world = World::new();
    for break_it in [0_u8, 1, 2, 3] {
        let temporary = TempDir::new().expect("tempdir");
        let mut participants = world.participants();
        match break_it {
            0 => participants[0].export.target_synced = false,
            1 => participants[0].export.parent_directory_synced = false,
            2 => participants[0].export.reopen_verified = false,
            _ => participants[0].export.source_binding_stable = false,
        }
        let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
        let mut window = barrier
            .freeze(&request("cp-1", 1_000), settled(), &world.clock)
            .expect("freeze");
        assert!(
            matches!(
                window.seal(
                    &request("cp-1", 1_000),
                    transfer_source(temporary.path()),
                    InstantMillis::from_millis(10),
                ),
                Err(BarrierError::Owner(CheckpointError::Unverified {
                    owner: CheckpointOwner::Broker
                }))
            ),
            "break_it={break_it}"
        );
    }
}

#[test]
fn a_broker_watermark_that_still_reports_unresolved_blocks_the_seal() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let mut participants = world.participants();
    participants[0].export = verified_checkpoint(CheckpointOwner::Broker, world.vault, world.pair);
    if let OwnerWatermark::Broker { unresolved, .. } = &mut participants[0].export.watermark {
        *unresolved = true;
    }
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    assert!(matches!(
        window.seal(
            &request("cp-1", 1_000),
            transfer_source(temporary.path()),
            InstantMillis::from_millis(10),
        ),
        Err(BarrierError::BrokerUnresolved)
    ));
}

#[test]
fn a_seal_ticket_carries_every_present_owner_and_the_full_roster() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let participants = world.participants();
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let ticket = window
        .seal(
            &request("cp-1", 1_000),
            transfer_source(temporary.path()),
            InstantMillis::from_millis(10),
        )
        .expect("seal");

    assert_eq!(ticket.owners.len(), 6);
    assert_eq!(ticket.roster, full_roster());
    for owner in CheckpointOwner::FREEZE_ORDER {
        assert!(ticket.owner(owner).is_some(), "{owner:?}");
    }
}

#[test]
fn a_transfer_source_inside_the_vault_or_workbench_data_is_refused() {
    let temporary = TempDir::new().expect("tempdir");
    let vault = temporary.path().join("vault");
    let inside = vault.join("staging");
    let mut source = transfer_source(&inside);
    source.expires_at = InstantMillis::from_millis(1);

    let error = source.validate(
        &vault,
        &temporary.path().join("workbench"),
        InstantMillis::from_millis(10),
    );

    assert_eq!(error, Err(SealError::TransferSourceInsideProtectedRoot));
}

#[test]
fn a_hard_linked_active_file_is_not_an_immutable_view() {
    let temporary = TempDir::new().expect("tempdir");
    let mut source = transfer_source(temporary.path());
    source.materialization = studiolo_backup::checkpoint::types::StagingMaterialization::HardLink;

    assert_eq!(
        source.validate(
            std::path::Path::new("/vault"),
            std::path::Path::new("/workbench"),
            InstantMillis::from_millis(99_999),
        ),
        Err(SealError::HardLinkIsNotImmutable)
    );
}

#[test]
fn a_transfer_source_outliving_the_shortest_retention_would_be_a_hidden_recovery_point() {
    let temporary = TempDir::new().expect("tempdir");
    let source = transfer_source(temporary.path());

    assert_eq!(
        source.validate(
            std::path::Path::new("/vault"),
            std::path::Path::new("/workbench"),
            InstantMillis::from_millis(4_999),
        ),
        Err(SealError::TransferSourceOutlivesRetention)
    );
}
