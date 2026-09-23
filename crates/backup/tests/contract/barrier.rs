//! 行为与失败契约：跨 owner 屏障必须固定顺序、有界等待、逐项 present/absent，
//! 并且拒绝一切「看起来像冻结」的替代物。
//!
//! 本文件管登记与固定顺序（含 M4 策略库的位置）；排空与有界等待在 `drain`，
//! 导出、封存与传输源在 `seal`。

mod drain;
mod seal;

use super::support::{StubParticipant, TestClock, request};
use std::sync::atomic::Ordering;
use studiolo_backup::checkpoint::barrier::{BarrierError, BrokerPrecondition, CrossOwnerBarrier};
use studiolo_core::VaultUuid;
use studiolo_core::checkpoint::{
    CheckpointError, CheckpointOwner, CheckpointParticipant, OwnerAssembly,
};

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

/// 某个 owner 在固定顺序里的下标；按 owner 取参与者，不写死下标。
fn position(owner: CheckpointOwner) -> usize {
    CheckpointOwner::FREEZE_ORDER
        .iter()
        .position(|candidate| *candidate == owner)
        .expect("owner 必须在固定顺序里")
}

#[test]
fn every_assembled_owner_freezes_in_the_fixed_order_and_the_roster_lists_all_seven() {
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
        7,
        "七个 owner 一个都不能缺项"
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
    assert_eq!(
        window.receipts().len(),
        6,
        "七个 owner 里只有未装配的那个没有回执"
    );
    window.thaw();
}

#[test]
fn the_policy_store_freezes_right_after_the_agent_store_and_the_audit_still_goes_last() {
    assert_eq!(
        CheckpointOwner::FREEZE_ORDER,
        [
            CheckpointOwner::Broker,
            CheckpointOwner::Control,
            CheckpointOwner::Auth,
            CheckpointOwner::Bridge,
            CheckpointOwner::AgentStore,
            CheckpointOwner::Policy,
            CheckpointOwner::Audit,
        ],
        "策略库在 AgentStore 之后、Audit 之前进入冻结"
    );
    assert_eq!(
        CheckpointOwner::FREEZE_ORDER.last(),
        Some(&CheckpointOwner::Audit)
    );
    assert_eq!(CheckpointOwner::Policy.as_str(), "policy");
    assert_eq!(
        serde_json::to_value(CheckpointOwner::Policy).expect("encode"),
        serde_json::json!("policy"),
        "清单里的稳定名字与序列化名一致"
    );
}

#[test]
fn a_barrier_without_the_policy_store_is_refused_rather_than_silently_skipping_it() {
    let world = World::new();
    let mut participants = world.participants();
    participants.retain(|participant| participant.owner != CheckpointOwner::Policy);

    assert!(matches!(
        CrossOwnerBarrier::assemble(refs(&participants)).err(),
        Some(BarrierError::Owner(CheckpointError::OwnerMissing {
            owner: CheckpointOwner::Policy
        }))
    ));
}

#[test]
fn an_undrained_policy_store_stops_the_freeze_before_the_audit_is_frozen() {
    let world = World::new();
    let mut participants = world.participants();
    let policy = position(CheckpointOwner::Policy);
    participants[policy].drain.in_flight_operations = 1;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");

    assert!(matches!(
        barrier.freeze(&request("cp-1", 1_000), settled(), &world.clock),
        Err(BarrierError::Owner(CheckpointError::NotDrained {
            owner: CheckpointOwner::Policy
        }))
    ));
    for participant in &participants[..=policy] {
        assert_eq!(
            participant.thawed.load(Ordering::SeqCst),
            1,
            "{:?} 必须被解冻",
            participant.owner
        );
    }
    assert_eq!(
        participants[position(CheckpointOwner::Audit)]
            .thawed
            .load(Ordering::SeqCst),
        0,
        "Audit 还没进入存储冻结，不该被解冻"
    );
}

#[test]
fn an_unassembled_policy_store_is_listed_as_absent_and_never_touched() {
    // 接入方未给 `--policy` 的 M3 兼容模式：策略库未装配，清单必须显式标记 absent。
    let world = World::new();
    let mut participants = world.participants();
    let policy = position(CheckpointOwner::Policy);
    participants[policy].assembly = OwnerAssembly::Absent;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let roster = window.roster().expect("roster");
    assert_eq!(roster.entries().count(), 7);
    assert_eq!(
        roster.assembly(CheckpointOwner::Policy),
        OwnerAssembly::Absent
    );
    assert!(
        window
            .receipts()
            .iter()
            .all(|receipt| receipt.owner != CheckpointOwner::Policy)
    );
    window.thaw();
    assert_eq!(
        participants[policy].thawed.load(Ordering::SeqCst),
        0,
        "备份器不得擅自停止或解冻未装配的策略库"
    );
}
