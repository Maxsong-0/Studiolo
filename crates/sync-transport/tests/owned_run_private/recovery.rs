//! 恢复契约：在每个阶段边界前后各切一刀，重开只观察、不重新执行、不假造原成功。

use super::support::*;
use crate::linux::{ContainmentGap, IdentityMatch};
use crate::owned_run::*;

/// 把一次运行推进到指定发布次数后中断，返回中断时的活动记录。
fn cut(fixture: &Fixture, publishes_before_failure: u32) -> RunRecord {
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    fixture
        .slot
        .fail_replace_after(publishes_before_failure, TransportError::Unavailable);
    assert!(session.start(&prepared).is_err());
    fixture.slot.state().active.expect("原槽保留")
}

#[test]
fn a_crash_right_after_preparation_reopens_as_prepared_and_executes_nothing() {
    let fixture = Fixture::new();
    {
        let mut session = fixture.session();
        session
            .prepare_once(1, RunOperation::RunOnce, 1)
            .expect("可以准备一次运行");
    }
    let mut reopened = fixture.session();
    let status = reopened.inspect().expect("可以读状态");
    let active = status.active.expect("活动槽仍在");
    assert_eq!(active.observation.phase, RunPhase::Prepared);
    assert!(status.audit_pending);
    assert_eq!(reopened.backend.launches, 0);
    assert_eq!(
        reopened.recover(active.run_id).expect("可以只观察").phase,
        RunPhase::Prepared
    );
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_crash_between_launch_issued_and_contained_reopens_without_a_second_execution() {
    let fixture = Fixture::new();
    let record = cut(&fixture, 1);
    assert_eq!(record.phase, RunPhase::LaunchIssued);
    assert!(record.container.is_none());

    let mut reopened = fixture.session();
    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::LaunchIssued);
    assert_eq!(observation.quiescence, Quiescence::Unconfirmed);
    assert_eq!(observation.exit, ExitObservation::NotObserved);
    assert_eq!(reopened.backend.launches, 0);
    assert_eq!(reopened.backend.releases, 0);
}

#[test]
fn a_crash_between_contained_and_execution_issued_still_allows_stopping_the_old_container() {
    let fixture = Fixture::new();
    let record = cut(&fixture, 2);
    assert_eq!(record.phase, RunPhase::Contained);
    assert!(record.container.is_some());

    let mut reopened = fixture.session();
    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::Contained);
    assert_eq!(observation.quiescence, Quiescence::Unconfirmed);
    assert_eq!(reopened.backend.releases, 0);

    reopened
        .stop_recovered(record.run_id)
        .expect("完整匹配旧容器时可以显式停止");
    assert_eq!(reopened.backend.stops, 1);
    assert_eq!(reopened.backend.releases, 0);
    let stopped = fixture.slot.state().active.expect("原槽保留");
    assert_eq!(stopped.phase, RunPhase::StopRequested);
    assert_eq!(stopped.stop_reason, Some(StopReason::OwnerRequested));
}

#[test]
fn a_crash_right_after_execution_issued_never_replays_the_release() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    session.backend.release_gap = Some(ContainmentGap::ManagerUnavailable);
    assert!(session.start(&prepared).is_err());
    let record = fixture.slot.state().active.expect("原槽保留");
    assert_eq!(record.phase, RunPhase::ExecutionIssued);

    let mut reopened = fixture.session();
    reopened.backend.running = false;
    reopened.backend.populated = true;
    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.quiescence, Quiescence::Unconfirmed);
    assert_eq!(reopened.backend.launches, 0);
    assert_eq!(reopened.backend.releases, 0);
}

#[test]
fn a_main_process_exit_with_surviving_descendants_yields_no_quiescent_capability() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    // 主进程 exit 0，但同一子组里仍有后代在写镜像。
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = true;
    session.wait(&owned).expect("可以推进观察");

    assert_eq!(
        session.quiescent().err(),
        Some(TransportError::AuditPending)
    );
    session
        .request_stop(&owned, StopReason::OwnerRequested)
        .expect("只停自己的容器");
    assert_eq!(session.backend.stops, 1);

    // 后代真正退出后才取得静止证明。
    session.backend.populated = false;
    let observation = session.wait(&owned).expect("可以推进观察");
    assert_eq!(observation.quiescence, Quiescence::Confirmed);
    assert_eq!(observation.stop_reason, Some(StopReason::OwnerRequested));
}

#[test]
fn a_crash_between_quiesced_and_the_fixed_outcome_reopens_and_finishes_without_re_execution() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    // wait 依次发布 RecordExit、Quiesce、FixOutcome；在 FixOutcome 之前切一刀。
    fixture
        .slot
        .fail_replace_after(2, TransportError::Unavailable);
    assert!(session.wait(&owned).is_err());
    let record = fixture.slot.state().active.expect("原槽保留");
    assert_eq!(record.phase, RunPhase::Quiesced);
    assert!(record.outcome.is_none());

    let mut reopened = fixture.session();
    reopened.backend.populated = false;
    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::OutcomePending);
    assert_eq!(observation.quiescence, Quiescence::Confirmed);
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_crash_between_the_fixed_outcome_and_its_acknowledgement_blocks_any_new_run() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Code(0);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");
    drop(session);

    let mut reopened = fixture.session();
    assert_eq!(
        reopened.prepare_once(2, RunOperation::RunOnce, 9).err(),
        Some(TransportError::RunActive)
    );
    assert_eq!(
        reopened.quiescent().err(),
        Some(TransportError::AuditPending)
    );
    let candidate = reopened
        .pending_audit()
        .expect("可以读待补事件")
        .expect("确有待补 Outcome");
    assert_eq!(candidate.phase(), AuditPhaseKind::Outcome);
    let outcome = candidate.payload().outcome.expect("Outcome 摘要存在");
    assert!(outcome.exit_observed);
    assert_eq!(outcome.quiescence, Quiescence::Confirmed);
    reopened
        .confirm_audit(&candidate, 2, [0xCD; 32])
        .expect("可以补齐原事件");
    reopened.quiescent().expect("闭合后才发静止能力");
}

#[test]
fn a_reopen_after_acknowledgement_still_reports_the_same_closed_terminal() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).expect("可以放行");
    session.backend.running = false;
    session.backend.exit = ExitObservation::Signal(15);
    session.backend.populated = false;
    session.wait(&owned).expect("可以推进观察");
    acknowledge(&mut session);
    drop(session);

    let reopened = fixture.session();
    let terminal = reopened
        .inspect()
        .expect("可以读状态")
        .last_terminal
        .expect("已审计闭合摘要");
    assert_eq!(terminal.observation.exit, ExitObservation::Signal(15));
    assert_eq!(terminal.observation.phase, RunPhase::Closed);
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_missing_or_replaced_unit_and_a_reused_cgroup_never_fabricate_the_original_success() {
    for identity in [
        IdentityMatch::UnitMissing,
        IdentityMatch::UnitReplaced,
        IdentityMatch::CgroupReused,
    ] {
        let fixture = Fixture::new();
        let record = cut(&fixture, 2);
        let mut reopened = fixture.session();
        reopened.backend.identity = identity;
        reopened.backend.populated = false;
        reopened.backend.exit = ExitObservation::Code(0);

        let observation = reopened.recover(record.run_id).expect("可以只观察");
        assert_eq!(observation.quiescence, Quiescence::Unconfirmed);
        assert_eq!(observation.exit, ExitObservation::NotObserved);
        assert_eq!(observation.phase, RunPhase::Contained);
        assert_eq!(
            reopened.stop_recovered(record.run_id).err(),
            Some(TransportError::ExitUnconfirmed)
        );
        assert_eq!(
            reopened.quiescent().err(),
            Some(TransportError::AuditPending)
        );
    }
}

#[test]
fn a_boot_change_proves_the_old_processes_are_gone_but_never_the_original_exit_result() {
    let fixture = Fixture::new();
    let record = cut(&fixture, 2);

    let mut reopened = fixture.session();
    reopened.backend.identity = IdentityMatch::BootChanged;
    reopened.backend.boot = [0x5A; 16];
    reopened.backend.exit = ExitObservation::Code(0);

    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::OutcomePending);
    assert_eq!(observation.quiescence, Quiescence::Confirmed);
    // 跨 boot 只能证明旧进程不存在；原退出结果永远未知。
    assert_eq!(observation.exit, ExitObservation::NotObserved);
    let proof = fixture
        .slot
        .state()
        .active
        .expect("原槽保留")
        .quiescence
        .expect("静止证明已持久");
    assert_eq!(proof.evidence, QuiescenceEvidence::BootChanged);
    assert_eq!(proof.cgroup_identity, None);
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_boot_change_before_any_container_identity_also_closes_the_slot_with_an_unknown_exit() {
    let fixture = Fixture::new();
    let record = cut(&fixture, 1);
    assert_eq!(record.phase, RunPhase::LaunchIssued);

    let mut reopened = fixture.session();
    reopened.backend.boot = [0x5A; 16];
    let observation = reopened.recover(record.run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::OutcomePending);
    assert_eq!(observation.quiescence, Quiescence::Confirmed);
    assert_eq!(observation.exit, ExitObservation::NotObserved);
    assert_eq!(reopened.backend.launches, 0);
}

#[test]
fn a_cancelled_run_that_later_reports_a_zero_exit_stays_cancelled_across_a_reopen() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = session
        .prepare_once(1, RunOperation::RunOnce, 1)
        .expect("可以准备一次运行");
    let run_id = prepared.run_id();
    session.cancel_prepared(prepared).expect("尚未启动可以取消");
    drop(session);

    let mut reopened = fixture.session();
    reopened.backend.exit = ExitObservation::Code(0);
    reopened.backend.populated = false;
    let observation = reopened.recover(run_id).expect("可以只观察");
    assert_eq!(observation.phase, RunPhase::Cancelled);
    assert_eq!(observation.exit, ExitObservation::NotObserved);
    assert_eq!(reopened.backend.launches, 0);
}
