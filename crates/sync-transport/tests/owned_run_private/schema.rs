//! 持久 schema v1 契约：完整记录、失败关闭、绝不自动迁移或部分修复。

use super::support::*;
use crate::owned_run::*;
use serde_json::{Value, json};

/// 跑完一次完整运行，取出 OutcomePending 时刻的权威字节。
fn full_run_bytes() -> Vec<u8> {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");
    fixture.slot.bytes().expect("槽中已有权威记录")
}

fn mutated(mutate: impl FnOnce(&mut Value)) -> Result<TransportState> {
    let mut value: Value = serde_json::from_slice(&full_run_bytes()).expect("权威记录是 JSON");
    mutate(&mut value);
    state::decode(&serde_json::to_vec(&value).expect("可以重新编码"))
}

#[test]
fn an_unknown_schema_version_is_reported_as_unknown_and_never_migrated() {
    for version in [0, 2, 255] {
        assert_eq!(
            mutated(|value| value["schema_version"] = json!(version)).err(),
            Some(TransportError::UnknownSchema),
            "schema_version={version}"
        );
    }
}

#[test]
fn unknown_fields_half_json_or_a_positional_array_fail_closed() {
    assert_eq!(
        mutated(|value| value["extra"] = json!(1)).err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(|value| value["active"]["extra"] = json!(1)).err(),
        Some(TransportError::CorruptStore)
    );
    let bytes = full_run_bytes();
    assert_eq!(
        state::decode(&bytes[..bytes.len() / 2]).err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        state::decode(b"[1,2,3]").err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(state::decode(b"").err(), Some(TransportError::CorruptStore));
    let mut trailing = full_run_bytes();
    trailing.extend_from_slice(b"{}");
    assert_eq!(
        state::decode(&trailing).err(),
        Some(TransportError::CorruptStore)
    );
}

#[test]
fn a_record_over_the_sixty_four_kib_budget_is_refused_in_both_directions() {
    let mut oversized = vec![b' '; STATE_MAX_BYTES + 1];
    oversized.extend_from_slice(b"{}");
    assert_eq!(
        state::decode(&oversized).err(),
        Some(TransportError::Budget)
    );

    let approval = FakeApproval::new();
    let runtime = PinnedRuntime::from_approval(&approval).expect("固定运行时可取");
    let mut state = initial_state(&approval, &runtime, OWNER).expect("首条记录可构造");
    state.backend.service_unit = "x".repeat(70_000);
    assert_eq!(state::encode(&state).err(), Some(TransportError::Budget));
}

#[test]
fn a_phase_without_its_supporting_evidence_is_refused() {
    // Quiesced 之后必须带真实静止证明。
    assert_eq!(
        mutated(|value| value["active"]["quiescence"] = json!(null)).err(),
        Some(TransportError::CorruptStore)
    );
    // 静止证明必须带「入口已关闭」。
    assert_eq!(
        mutated(|value| value["active"]["quiescence"]["spawn_gate_closed"] = json!(false)).err(),
        Some(TransportError::CorruptStore)
    );
    // 子组空证明必须绑定实测子组身份。
    assert_eq!(
        mutated(|value| value["active"]["quiescence"]["cgroup_identity"] = json!(null)).err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(|value| value["active"]["quiescence"]["cgroup_identity"] = json!([64, 1234])).err(),
        Some(TransportError::CorruptStore)
    );
    // OutcomePending 必须已有固定 Outcome 槽。
    assert_eq!(
        mutated(|value| value["active"]["outcome"] = json!(null)).err(),
        Some(TransportError::CorruptStore)
    );
    // Closed 必须已 ACK。
    assert_eq!(
        mutated(|value| {
            value["active"]["phase"] = json!("closed");
            value["active"]["outcome"]["receipt"] = json!(null);
        })
        .err(),
        Some(TransportError::CorruptStore)
    );
    // 已发出启动请求就必须留有当时的 boot 标识。
    assert_eq!(
        mutated(|value| value["active"]["requested_boot_id"] = json!(null)).err(),
        Some(TransportError::CorruptStore)
    );
}

#[test]
fn a_terminal_summary_can_never_collide_with_the_active_run_or_outrun_the_sequence() {
    assert_eq!(
        mutated(|value| {
            let active = value["active"].clone();
            value["last_terminal"] = json!({
                "run_id": active["run_id"],
                "sequence": active["sequence"],
                "operation": active["operation"],
                "observation": {
                    "phase": "closed",
                    "quiescence": "confirmed",
                    "exit": {"code": 0},
                    "stop_reason": null,
                },
                "counts": active["counts"],
            });
        })
        .err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(|value| value["active"]["sequence"] = json!(99)).err(),
        Some(TransportError::CorruptStore)
    );
}

#[test]
fn an_owner_identity_equal_to_the_payload_identity_is_never_isolation() {
    assert_eq!(
        mutated(|value| value["owner"]["uid"] = value["backend"]["payload_uid"].clone()).err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(
            |value| value["roots"]["mirror_identity"] = value["roots"]["private_identity"].clone()
        )
        .err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(|value| value["generation"] = json!(0)).err(),
        Some(TransportError::CorruptStore)
    );
    assert_eq!(
        mutated(|value| value["active"]["budget"]["pids_max"] = json!(0)).err(),
        Some(TransportError::CorruptStore)
    );
}

#[test]
fn a_binding_with_a_zero_scope_epoch_is_refused_before_any_slot_is_created() {
    assert_eq!(
        mutated(|value| value["binding"]["scope_epoch"] = json!(0)).err(),
        Some(TransportError::CorruptStore)
    );
    let approval = FakeApproval::new();
    let runtime = PinnedRuntime::from_approval(&approval).expect("固定运行时可取");
    let state = initial_state(&approval, &runtime, OWNER).expect("首条记录可构造");
    assert_eq!(state.schema_version, SCHEMA_VERSION);
    assert_eq!(state.generation, 1);
    assert_eq!(state.next_sequence, 1);
    assert!(state.active.is_none());
    assert!(state.last_terminal.is_none());
}

#[test]
fn a_slot_whose_binding_no_longer_matches_the_approval_is_refused_on_every_read() {
    let fixture = Fixture::new();
    let session = fixture.session();
    fixture.approval.binding.borrow_mut().link_id = [0x77; 16];
    assert_eq!(
        session.inspect().err(),
        Some(TransportError::BindingMismatch)
    );
}
