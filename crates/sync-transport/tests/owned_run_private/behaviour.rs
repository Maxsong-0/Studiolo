//! 行为契约：阶段持久边界、四项分开报告、静止能力的取得与作废。

use super::support::*;
use crate::owned_run::*;

#[test]
fn a_prepared_run_is_durable_before_any_launch_command_is_issued() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session
        .prepare_once(1, RunOperation::RunOnce, 1_700_000_000_000)
        .expect("可以准备一次运行");

    let state = fixture.slot.state();
    let record = state.active.as_ref().expect("活动槽已发布");
    assert_eq!(record.phase, RunPhase::Prepared);
    assert_eq!(record.run_id, prepared.run_id());
    assert!(record.container.is_none());
    assert!(record.requested_boot_id.is_none());
    assert!(record.intent.pending());
    assert_eq!(state.next_sequence, 2);
    assert!(session.inspect().expect("可以读状态").audit_pending);
    // 准备本身没有发出任何启动命令。
    assert_eq!(session.backend.launches, 0);
}

#[test]
fn intent_acknowledgement_is_required_before_the_launcher_is_ever_asked_to_start() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session
        .prepare_once(1, RunOperation::RunOnce, 1)
        .expect("可以准备一次运行");

    assert_eq!(session.start(&prepared), Err(TransportError::AuditPending));
    assert_eq!(session.backend.launches, 0);
    assert_eq!(
        fixture.slot.state().active.expect("活动槽仍在").phase,
        RunPhase::Prepared
    );
}

#[test]
fn a_full_run_reports_phase_quiescence_exit_and_stop_reason_as_four_separate_facts() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("Intent 已 ACK 后可以放行");
    assert_eq!(session.backend.launches, 1);
    assert_eq!(session.backend.releases, 1);

    // payload 仍在运行：静止未证明，退出结果未观察到。
    let observation = session.wait(&owned).expect("可以推进观察");
    assert_eq!(
        observation,
        RunObservation {
            phase: RunPhase::Running,
            quiescence: Quiescence::Unconfirmed,
            exit: ExitObservation::NotObserved,
            stop_reason: None,
        }
    );

    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    let observation = session.wait(&owned).expect("可以推进观察");
    assert_eq!(
        observation,
        RunObservation {
            phase: RunPhase::OutcomePending,
            quiescence: Quiescence::Confirmed,
            exit: ExitObservation::Code(0),
            stop_reason: None,
        }
    );
    // 静止证明必须绑定同一个受控子组。
    let proof = fixture
        .slot
        .state()
        .active
        .expect("活动槽仍在")
        .quiescence
        .expect("静止证明已持久");
    assert_eq!(proof.evidence, QuiescenceEvidence::PayloadCgroupUnpopulated);
    assert_eq!(proof.cgroup_identity, Some(container().cgroup_identity));
    assert!(proof.spawn_gate_closed);
}

#[test]
fn a_quiescent_capability_is_only_issued_after_confirmed_quiescence_and_outcome_acknowledgement() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");

    // Outcome 未 ACK 前不发静止能力。
    assert_eq!(
        session.quiescent().err(),
        Some(TransportError::AuditPending)
    );
    acknowledge(&mut session);

    let mirror = session.quiescent().expect("闭合后可取静止能力");
    assert_eq!(mirror.run_id(), owned.run_id());
    assert_eq!(mirror.mirror_root(), std::path::Path::new(MIRROR_ROOT));
    assert!(mirror.remote_delivery_unknown());
    assert_eq!(session.verify_quiescent(&mirror), Ok(()));

    let status = session.inspect().expect("可以读状态");
    assert!(status.active.is_none());
    let terminal = status.last_terminal.expect("已审计闭合摘要");
    assert_eq!(terminal.observation.phase, RunPhase::Closed);
    assert_eq!(terminal.observation.quiescence, Quiescence::Confirmed);
    assert!(!status.audit_pending);
}

#[test]
fn the_next_preparation_invalidates_a_previously_issued_quiescent_capability() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");
    acknowledge(&mut session);
    let mirror = session.quiescent().expect("闭合后可取静止能力");

    session
        .prepare_once(2, RunOperation::InspectConfig, 2)
        .expect("可以准备下一次运行");

    assert_eq!(
        session.verify_quiescent(&mirror),
        Err(TransportError::TicketStale)
    );
}

#[test]
fn a_cancelled_preparation_persists_a_fixed_not_executed_outcome_and_starts_nothing() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session
        .prepare_once(1, RunOperation::RunOnce, 1)
        .expect("可以准备一次运行");

    session.cancel_prepared(prepared).expect("尚未启动可以取消");
    let record = fixture.slot.state().active.expect("活动槽仍在");
    assert_eq!(record.phase, RunPhase::Cancelled);
    assert!(record.container.is_none());
    assert_eq!(record.exit, ExitObservation::NotObserved);
    assert!(record.quiescence.is_none());
    // 一个活动槽最多两个待补事件：Intent 与固定「未执行」Outcome。
    assert!(record.intent.pending());
    assert!(
        record
            .outcome
            .as_ref()
            .expect("固定 Outcome 已发布")
            .pending()
    );
    assert_eq!(session.backend.launches, 0);

    acknowledge(&mut session);
    acknowledge(&mut session);
    let status = session.inspect().expect("可以读状态");
    let terminal = status.last_terminal.expect("已审计闭合摘要");
    assert_eq!(terminal.observation.phase, RunPhase::Closed);
    // 什么都没跑过，就不会有静止证明，也不发静止能力。
    assert_eq!(terminal.observation.quiescence, Quiescence::Unconfirmed);
    assert_eq!(
        session.quiescent().err(),
        Some(TransportError::QuiescenceUnconfirmed)
    );
}

#[test]
fn sequence_numbers_advance_monotonically_and_are_never_reused() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session
        .prepare_once(1, RunOperation::RunOnce, 1)
        .expect("可以准备一次运行");
    session.cancel_prepared(prepared).expect("可以取消");
    acknowledge(&mut session);
    acknowledge(&mut session);

    assert_eq!(
        session.prepare_once(1, RunOperation::RunOnce, 2).err(),
        Some(TransportError::SequenceMismatch)
    );
    let second = session
        .prepare_once(2, RunOperation::RunOnce, 2)
        .expect("下一个序号可用");
    assert_eq!(second.sequence(), 2);
    assert_eq!(fixture.slot.state().next_sequence, 3);
}

#[test]
fn stop_reason_is_persisted_before_the_bound_subgroup_is_ever_asked_to_stop() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.wait(&owned).expect("可以推进观察");

    session
        .request_stop(&owned, StopReason::OwnerRequested)
        .expect("可以请求停止");
    let record = fixture.slot.state().active.expect("活动槽仍在");
    assert_eq!(record.phase, RunPhase::StopRequested);
    assert_eq!(record.stop_reason, Some(StopReason::OwnerRequested));
    assert_eq!(session.backend.stops, 1);
}
