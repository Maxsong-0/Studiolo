//! 恢复动手之前的两个纯判断（T-212）：只看清单定能不能恢复（名单和水位），和新根 data 里读到的 lineage 对这次
//! 恢复意味着什么（Codex 第二意见第 5 条：被封存的根本身是恢复出来的，它的 lineage 只当上一环）。
use crate::checkpoint::{Found, RestoreRefusal, classify, plan_from};
use studiolo_backup::checkpoint::types::OwnerRoster;
use studiolo_core::{
    VaultUuid,
    checkpoint::{AuditChainCutoff, CheckpointOwner as Owner, OwnerAssembly, OwnerWatermark},
};
use studiolo_vault_broker::{LineageState, PhysicalId, RestoreLineage};

fn roster(present: &[Owner]) -> OwnerRoster {
    OwnerRoster::new(Owner::FREEZE_ORDER.map(|owner| {
        let assembly = if present.contains(&owner) {
            OwnerAssembly::Present
        } else {
            OwnerAssembly::Absent
        };
        (owner, assembly)
    }))
    .unwrap()
}
fn cutoff(offset: u64) -> AuditChainCutoff {
    AuditChainCutoff {
        last_offset: offset,
        last_hash: "ab".repeat(32),
    }
}
fn broker(vault: VaultUuid, pair: VaultUuid, unresolved: bool) -> OwnerWatermark {
    OwnerWatermark::Broker {
        vault_uuid: vault,
        pair_id: pair,
        committed_through: 3,
        unresolved,
    }
}
fn auth() -> OwnerWatermark {
    OwnerWatermark::Auth {
        schema_version: 4,
        owner_binding_digest: "cd".repeat(32),
        audit_anchor_offset: 2,
    }
}
fn audit(offset: u64, agent: Option<AuditChainCutoff>) -> OwnerWatermark {
    OwnerWatermark::Audit {
        last_offset: offset,
        last_hash: "ab".repeat(32),
        agent_ledger: agent,
    }
}
fn control(vault: VaultUuid) -> OwnerWatermark {
    OwnerWatermark::Control {
        schema_version: 4,
        vault_uuid: vault,
        outbox_pending: 0,
        outbox_delivered_through: 1,
    }
}
fn agents() -> OwnerWatermark {
    OwnerWatermark::AgentStore {
        schema_version: 1,
        type_token_digest: "ef".repeat(32),
        instance_count: 1,
    }
}
fn policy() -> OwnerWatermark {
    OwnerWatermark::Policy {
        schema_version: 1,
        effective_version: 2,
        effective_hash: "01".repeat(32),
        global_revoked: false,
    }
}
fn bridge(closed: bool) -> OwnerWatermark {
    OwnerWatermark::Bridge {
        link_id: "link".into(),
        scope_digest: "02".repeat(32),
        common_base_digest: "03".repeat(32),
        observation_sequence: 4,
        durable_gate_closed: closed,
    }
}
const HUMAN: [Owner; 3] = [Owner::Broker, Owner::Auth, Owner::Audit];

#[test]
fn a_human_only_roster_plans_the_three_human_components() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let plan = plan_from(
        &roster(&HUMAN),
        &[broker(vault, pair, false), auth(), audit(5, None)],
    )
    .unwrap();
    assert_eq!((plan.vault_uuid, plan.pair_id), (vault, pair));
    assert_eq!(plan.audit, cutoff(5));
    assert!(plan.agent_audit.is_none() && plan.control.is_none());
    assert!(plan.agents.is_none() && plan.policy.is_none() && !plan.bridge);
    assert_eq!(plan.restored_owners(), HUMAN);
}

#[test]
fn a_bridge_with_its_gate_closed_is_left_closed_and_never_placed() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let mut present = HUMAN.to_vec();
    present.push(Owner::Bridge);
    let plan = plan_from(
        &roster(&present),
        &[
            broker(vault, pair, false),
            bridge(true),
            auth(),
            audit(5, None),
        ],
    )
    .unwrap();
    assert!(plan.bridge);
    // Bridge 的运行状态不恢复，不算进要失效的 owner。
    assert_eq!(plan.restored_owners(), HUMAN);
}

#[test]
fn a_full_board_roster_plans_every_owner_with_its_watermark() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let present = [
        Owner::Broker,
        Owner::Control,
        Owner::Auth,
        Owner::AgentStore,
        Owner::Policy,
        Owner::Audit,
    ];
    let plan = plan_from(
        &roster(&present),
        &[
            broker(vault, pair, false),
            control(vault),
            auth(),
            agents(),
            policy(),
            audit(9, Some(cutoff(2))),
        ],
    )
    .unwrap();
    assert_eq!(plan.control, Some(control(vault)));
    assert_eq!(plan.agents, Some(agents()));
    assert_eq!(plan.policy, Some(policy()));
    assert_eq!(plan.agent_audit, Some(cutoff(2)));
    assert_eq!(plan.restored_owners(), present);
}

#[test]
fn rosters_that_do_not_add_up_are_refused_before_anything_is_written() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let human = [broker(vault, pair, false), auth(), audit(5, None)];
    let with = |extra: &[OwnerWatermark]| {
        let mut marks = human.to_vec();
        marks.extend_from_slice(extra);
        marks
    };
    let board = [
        Owner::Broker,
        Owner::Auth,
        Owner::AgentStore,
        Owner::Policy,
        Owner::Audit,
    ];
    let cases: Vec<(&str, OwnerRoster, Vec<OwnerWatermark>)> = vec![
        (
            "Auth 缺席",
            roster(&[Owner::Broker, Owner::Audit]),
            with(&[]),
        ),
        (
            "Broker 还有没处理完的",
            roster(&HUMAN),
            vec![broker(vault, pair, true), auth(), audit(5, None)],
        ),
        (
            "人侧账本是空的",
            roster(&HUMAN),
            vec![broker(vault, pair, false), auth(), audit(0, None)],
        ),
        (
            "缺 Auth 水位",
            roster(&HUMAN),
            vec![broker(vault, pair, false), audit(5, None)],
        ),
        (
            "名单没有 Control 却有它的水位",
            roster(&HUMAN),
            with(&[control(vault)]),
        ),
        (
            "名单有 Control 却没有它的水位",
            roster(&[Owner::Broker, Owner::Control, Owner::Auth, Owner::Audit]),
            with(&[]),
        ),
        (
            "Bridge 的持久门禁开着",
            roster(&[Owner::Broker, Owner::Bridge, Owner::Auth, Owner::Audit]),
            with(&[bridge(false)]),
        ),
        (
            "有 AgentStore 却没有策略库",
            roster(&[Owner::Broker, Owner::Auth, Owner::AgentStore, Owner::Audit]),
            vec![
                broker(vault, pair, false),
                auth(),
                agents(),
                audit(5, Some(cutoff(1))),
            ],
        ),
        (
            "有 AgentStore 却没有接入方账本",
            roster(&board),
            with(&[agents(), policy()]),
        ),
        (
            "没有 AgentStore 却有接入方账本",
            roster(&HUMAN),
            vec![
                broker(vault, pair, false),
                auth(),
                audit(5, Some(cutoff(1))),
            ],
        ),
        (
            "同一个 owner 的水位出现两次",
            roster(&HUMAN),
            with(&[auth()]),
        ),
    ];
    for (case, roster, marks) in cases {
        assert_eq!(
            plan_from(&roster, &marks).unwrap_err(),
            RestoreRefusal::Roster,
            "{case}"
        );
    }
}

fn lineage(
    vault: VaultUuid,
    pair: VaultUuid,
    checkpoint: &str,
    restore_id: Option<&str>,
    state: LineageState,
) -> RestoreLineage {
    let id = |ino| PhysicalId { dev: 1, ino };
    let done = state == LineageState::Completed;
    RestoreLineage {
        format_version: 3,
        state,
        vault_uuid: vault,
        pair_id: pair,
        checkpoint_id: checkpoint.into(),
        previous_canonical: id(1),
        previous_data: id(2),
        restored_canonical: done.then(|| id(3)),
        restored_data: done.then(|| id(4)),
        parent_lineage: Some("drill-0".into()),
        restore_id: restore_id.map(str::to_owned),
        earlier_restores: Vec::new(),
    }
}

#[test]
fn the_sealed_roots_own_lineage_is_only_the_previous_link() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let decide = |found| classify(found, "cp-2", "drill-2", vault, pair);
    // data 里没有 lineage：这次的重绑定还没开始，没有上一环。
    assert_eq!(decide(None).unwrap(), Found::Fresh { parent: None });
    // 被封存的根是恢复出来的：别的 checkpoint 的、已完成的 lineage 只当上一环（以前这里整次拒绝）。
    let ancestor = lineage(
        vault,
        pair,
        "cp-1",
        Some("drill-1"),
        LineageState::Completed,
    );
    assert_eq!(
        decide(Some(ancestor)).unwrap(),
        Found::Fresh {
            parent: Some("drill-1".into())
        }
    );
    // 上一环的标识就是这次的也照样当上一环：复用由 Broker 按那一串拒绝，不在这里。
    let same_id = lineage(
        vault,
        pair,
        "cp-1",
        Some("drill-2"),
        LineageState::Completed,
    );
    assert_eq!(
        decide(Some(same_id)).unwrap(),
        Found::Fresh {
            parent: Some("drill-2".into())
        }
    );
    // 这次恢复自己写的：停在重绑定中途的接着绑，做完的不再重绑。
    let interrupted = lineage(
        vault,
        pair,
        "cp-2",
        Some("drill-2"),
        LineageState::Rebinding,
    );
    assert_eq!(
        decide(Some(interrupted)).unwrap(),
        Found::Interrupted {
            parent: Some("drill-0".into())
        }
    );
    let done = lineage(
        vault,
        pair,
        "cp-2",
        Some("drill-2"),
        LineageState::Completed,
    );
    assert_eq!(
        decide(Some(done.clone())).unwrap(),
        Found::Done(Box::new(done))
    );
}

#[test]
fn lineages_that_do_not_belong_to_this_restore_are_refused() {
    let (vault, pair) = (VaultUuid::new(), VaultUuid::new());
    let decide = |found| classify(Some(found), "cp-2", "drill-2", vault, pair);
    for (case, found) in [
        (
            "别的 Vault",
            lineage(
                VaultUuid::new(),
                pair,
                "cp-1",
                Some("drill-1"),
                LineageState::Completed,
            ),
        ),
        (
            "别的配对",
            lineage(
                vault,
                VaultUuid::new(),
                "cp-1",
                Some("drill-1"),
                LineageState::Completed,
            ),
        ),
        (
            "别的 checkpoint、停在重绑定中途",
            lineage(
                vault,
                pair,
                "cp-1",
                Some("drill-1"),
                LineageState::Rebinding,
            ),
        ),
        (
            "别的 checkpoint、第 1 版没有恢复标识",
            lineage(vault, pair, "cp-1", None, LineageState::Completed),
        ),
        (
            "同一个 checkpoint、别的恢复标识",
            lineage(
                vault,
                pair,
                "cp-2",
                Some("drill-3"),
                LineageState::Completed,
            ),
        ),
    ] {
        assert_eq!(decide(found).unwrap_err(), RestoreRefusal::Rebind, "{case}");
    }
}
