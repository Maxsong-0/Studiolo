//! 失败契约：任何缺证据都不得被当成好结果，任何启动后错误都不得声称从未执行。

use super::support::*;
use crate::linux::ContainmentGap;
use crate::owned_run::*;

#[test]
fn a_borrowed_preparation_ticket_can_only_release_execution_once() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    session.start(&prepared).expect("第一次可以放行");

    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::AlreadyStarted)
    );
    assert_eq!(session.backend.launches, 1);
    assert_eq!(session.backend.releases, 1);
}

#[test]
fn a_launch_error_after_launch_issued_never_claims_the_run_never_executed() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    session.backend.launch_gap = Some(ContainmentGap::ManagerUnavailable);

    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::ExitUnconfirmed)
    );
    let record = fixture.slot.state().active.expect("原槽保留");
    assert_eq!(record.phase, RunPhase::LaunchIssued);
    assert!(record.requested_boot_id.is_some());
    assert!(record.phase.execution_may_have_started());
    // 重开也不得二次 exec：活动槽还在，新的准备一律拒绝。
    let mut reopened = fixture.session();
    assert_eq!(
        reopened.prepare_once(2, RunOperation::RunOnce, 3).err(),
        Some(TransportError::RunActive)
    );
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_lost_release_acknowledgement_never_produces_a_second_execution() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    session.backend.release_gap = Some(ContainmentGap::ManagerUnavailable);

    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::ExitUnconfirmed)
    );
    let record = fixture.slot.state().active.expect("原槽保留");
    assert_eq!(record.phase, RunPhase::ExecutionIssued);
    assert!(record.phase.release_consumed());
    assert_eq!(session.backend.launches, 1);
    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::AlreadyStarted)
    );
    assert_eq!(session.backend.launches, 1);

    let mut reopened = fixture.session();
    reopened.recover(record.run_id).expect("重开只观察旧容器");
    assert_eq!(reopened.backend.launches, 0);
    assert_eq!(reopened.backend.releases, 0);
}

#[test]
fn quiescence_is_never_inferred_from_a_populated_subgroup_or_a_missing_exit_result() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    // 主进程已退出，但后代仍在同一子组里：不得声称静止。
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = true;

    let observation = session.wait(&owned).expect("可以推进观察");
    assert_eq!(observation.quiescence, Quiescence::Unconfirmed);
    assert_eq!(observation.exit, ExitObservation::Code(0));
    assert_ne!(observation.phase, RunPhase::Quiesced);
    assert!(
        fixture
            .slot
            .state()
            .active
            .expect("原槽保留")
            .quiescence
            .is_none()
    );
    assert_eq!(
        session.quiescent().err(),
        Some(TransportError::AuditPending)
    );
}

#[test]
fn a_recorded_cancellation_survives_a_later_zero_exit_code() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.wait(&owned).expect("可以推进观察");
    session
        .request_stop(&owned, StopReason::OwnerRequested)
        .expect("可以请求停止");
    // 后来的其它停止原因不覆盖第一个被持久的原因。
    session
        .request_stop(&owned, StopReason::ContainmentLost)
        .expect("重复请求被接受但不改写原因");

    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    let observation = session.wait(&owned).expect("可以推进观察");

    assert_eq!(observation.exit, ExitObservation::Code(0));
    assert_eq!(observation.stop_reason, Some(StopReason::OwnerRequested));
    assert_eq!(observation.phase, RunPhase::OutcomePending);
}

#[test]
fn output_over_budget_forces_a_stop_and_forbids_parsing_the_output() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.counts = ResourceCounts {
        stdout_bytes: 262_145,
        stderr_bytes: 0,
        output_over_budget: true,
    };

    let observation = session.wait(&owned).expect("可以推进观察");
    assert_eq!(
        observation.stop_reason,
        Some(StopReason::OutputBudgetExceeded)
    );
    assert_eq!(observation.phase, RunPhase::StopRequested);
    assert!(session.backend.stops >= 1);
    let record = fixture.slot.state().active.expect("原槽保留");
    assert!(record.counts.output_over_budget);
}

#[test]
fn tickets_from_another_owner_instance_are_refused_without_touching_the_slot() {
    let fixture = Fixture::new();
    let mut first = fixture.session();
    let prepared = fixture.prepared(&mut first);

    let mut second = fixture.session();
    assert_eq!(
        second.start(&prepared).err(),
        Some(TransportError::TicketForeign)
    );
    assert_eq!(second.backend.launches, 0);
    assert_eq!(
        fixture.slot.state().active.expect("原槽保留").phase,
        RunPhase::Prepared
    );
}

#[test]
fn a_configuration_or_root_change_invalidates_every_read_and_every_ticket() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);

    fixture.approval.roots.borrow_mut().mirror_identity = (64, 77);
    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::RootsChanged)
    );
    fixture.approval.roots.borrow_mut().mirror_identity = (64, 12);

    *fixture.approval.fresh.borrow_mut() = false;
    assert_eq!(
        session.start(&prepared).err(),
        Some(TransportError::ConfigurationChanged)
    );
    assert_eq!(session.backend.launches, 0);
}

#[test]
fn a_second_run_cannot_be_prepared_while_an_outcome_is_still_waiting_for_acknowledgement() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");

    assert_eq!(
        session.prepare_once(2, RunOperation::RunOnce, 5).err(),
        Some(TransportError::RunActive)
    );
    acknowledge(&mut session);
    session
        .prepare_once(2, RunOperation::RunOnce, 5)
        .expect("Outcome 已 ACK 后可以再准备");
}

#[test]
fn an_audit_confirmation_that_does_not_match_the_pending_event_is_rejected() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    session
        .prepare_once(1, RunOperation::RunOnce, 1)
        .expect("可以准备一次运行");
    let candidate = session
        .pending_audit()
        .expect("可以读待补事件")
        .expect("确有待补事件");
    assert_eq!(candidate.action(), TRANSPORT_RUN_ACTION);
    assert_eq!(candidate.phase(), AuditPhaseKind::Intent);
    assert!(candidate.payload().outcome.is_none());

    assert_eq!(
        session.confirm_audit(&candidate, 0, [1u8; 32]).err(),
        Some(TransportError::AuditRejected)
    );
    assert_eq!(
        session.confirm_audit(&candidate, 5, [0u8; 32]).err(),
        Some(TransportError::AuditRejected)
    );
    session
        .confirm_audit(&candidate, 5, [1u8; 32])
        .expect("首次确认可以登记");
    // 同一条事件不得被重复确认。
    assert_eq!(
        session.confirm_audit(&candidate, 6, [1u8; 32]).err(),
        Some(TransportError::AuditMismatch)
    );

    let prepared_ticket = session.load().expect("可以读权威记录");
    assert!(!prepared_ticket.active.expect("活动槽仍在").intent.pending());
}

#[test]
fn a_failed_publish_leaves_the_authoritative_record_at_the_previous_version() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let before = fixture.slot.state();
    fixture.slot.fail_next_replace(TransportError::Unavailable);

    assert_eq!(
        session.prepare_once(1, RunOperation::RunOnce, 1).err(),
        Some(TransportError::Unavailable)
    );
    let after = fixture.slot.state();
    assert_eq!(after, before);
    assert!(after.active.is_none());
    assert_eq!(after.generation, 1);
}

#[test]
fn a_container_outside_the_approved_unit_or_cgroup_prefix_is_never_released() {
    for mutate in [
        (|backend: &mut FakeBackend| backend.container.unit_name = "other.service".to_owned())
            as fn(&mut FakeBackend),
        |backend: &mut FakeBackend| {
            backend.container.payload_cgroup = "/sys/fs/cgroup/x".to_owned()
        },
        |backend: &mut FakeBackend| backend.container.cgroup_identity = (0, 0),
        |backend: &mut FakeBackend| backend.container.launcher_session = [0u8; 16],
        |backend: &mut FakeBackend| backend.container.boot_id = [0u8; 16],
    ] {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        mutate(&mut session.backend);

        assert_eq!(
            session.start(&prepared).err(),
            Some(TransportError::ExitUnconfirmed)
        );
        assert_eq!(session.backend.releases, 0);
        assert_eq!(
            fixture.slot.state().active.expect("原槽保留").phase,
            RunPhase::LaunchIssued
        );
    }
}

#[test]
fn actual_limits_or_isolation_short_of_the_policy_stop_the_run_before_the_barrier_opens() {
    let cases: [fn(&mut FakeBackend); 5] = [
        |backend| backend.limits.memory_max = None,
        |backend| backend.limits.memory_swap_max = Some(1),
        |backend| backend.limits.pids_max = Some(4096),
        |backend| backend.isolation.mount_namespace = false,
        |backend| backend.isolation.disk_quota_enforced = false,
    ];
    for mutate in cases {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        mutate(&mut session.backend);

        assert_eq!(
            session.start(&prepared).err(),
            Some(TransportError::ExitUnconfirmed)
        );
        assert_eq!(session.backend.releases, 0);
    }
}
