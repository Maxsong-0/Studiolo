//! 排空与有界等待：只暂停 HTTP、只成功补记一次、超出预算都不算冻结；
//! 任一 owner 失败就按逆序解冻已冻结的 owner，Broker 前置条件不成立则根本不开始。

use super::super::support::{drained, request};
use super::{World, refs, settled};
use studiolo_backup::checkpoint::barrier::{BarrierError, BrokerPrecondition, CrossOwnerBarrier};
use studiolo_core::checkpoint::{CheckpointError, CheckpointOwner};

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
    for participant in participants.iter().skip(4) {
        assert_eq!(
            participant.thawed.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{:?} 还没冻结，不该被解冻",
            participant.owner
        );
    }
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
