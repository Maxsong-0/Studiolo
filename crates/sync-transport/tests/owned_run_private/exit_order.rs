//! BUG-021：退出码与空组观察不可互相替代，先关门，保留待退出观察的运行槽。

use super::support::*;
use crate::owned_run::*;

#[test]
fn a_process_that_exits_before_the_first_wait_is_gated_before_reaping() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let prepared = fixture.prepared(&mut session);
    let owned = session.start(&prepared).unwrap();
    session.backend.running = false;
    session.backend.populated = false;
    session.backend.exit = ExitObservation::Code(0);
    let observed = session.wait(&owned).expect("快速退出仍须先关门再reap");
    assert_eq!(observed.phase, RunPhase::OutcomePending);
    assert_eq!(observed.exit, ExitObservation::Code(0));
    assert!(session.backend.gate_closed);
}

#[test]
fn an_empty_cgroup_does_not_close_a_run_before_its_exit_result_arrives() {
    for exit in [ExitObservation::Code(0), ExitObservation::Signal(15)] {
        let fixture = Fixture::new();
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        let owned = session.start(&prepared).unwrap();
        session.backend.running = false;
        session.backend.populated = false;
        session.backend.exit_after_events = Some(exit);
        let pending = session.wait(&owned).unwrap();
        assert_eq!(pending.phase, RunPhase::ExecutionIssued);
        assert_eq!(pending.exit, ExitObservation::NotObserved);
        assert_eq!(pending.quiescence, Quiescence::Unconfirmed);
        assert!(fixture.slot.state().active.unwrap().outcome.is_none());
        assert!(
            session
                .prepare_once(2, RunOperation::InspectConfig, 2)
                .is_err()
        );
        let observed = session.wait(&owned).unwrap();
        assert_eq!(observed.phase, RunPhase::OutcomePending);
        assert_eq!(observed.exit, exit);
        assert_eq!(observed.quiescence, Quiescence::Confirmed);
        acknowledge(&mut session);
        assert!(session.inspect().unwrap().active.is_none());
    }
}

#[test]
fn a_reopened_run_waits_for_the_same_exit_result_without_relaunching() {
    let fixture = Fixture::new();
    let run_id = {
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        session.start(&prepared).unwrap().run_id()
    };
    let mut reopened = fixture.session();
    reopened.backend.running = false;
    reopened.backend.populated = false;
    let pending = reopened.recover(run_id).unwrap();
    assert_eq!(pending.exit, ExitObservation::NotObserved);
    assert_eq!(pending.phase, RunPhase::ExecutionIssued);
    reopened.backend.exit = ExitObservation::Code(0);
    let observed = reopened.recover(run_id).unwrap();
    assert_eq!(observed.exit, ExitObservation::Code(0));
    assert_eq!(observed.phase, RunPhase::OutcomePending);
    assert_eq!(reopened.backend.launches, 0);
    assert_eq!(reopened.backend.releases, 0);
}

#[cfg(target_os = "linux")]
#[test]
fn the_real_adapter_keeps_an_unreaped_session_and_then_can_start_the_next_run() {
    use super::launcher_fake::FakeLauncher;
    use crate::linux::{LinuxContainment, verify_backend_at};
    use crate::owned_run::session::RunSession;

    let launcher = FakeLauncher::start();
    let fixture = Fixture::new();
    let backend = verify_backend_at(&fixture.approval, launcher.socket()).unwrap();
    let mut session = RunSession::open(
        &fixture.approval,
        PinnedRuntime::from_approval(&fixture.approval).unwrap(),
        LinuxContainment::new(backend),
        fixture.slot.publisher(),
    )
    .unwrap();
    let prepared = session.prepare_once(1, RunOperation::RunOnce, 1).unwrap();
    acknowledge(&mut session);
    let owned = session.start(&prepared).unwrap();
    {
        let mut script = launcher.script();
        script.running = false;
        script.populated = false;
    }
    assert_eq!(
        session.wait(&owned).unwrap().phase,
        RunPhase::ExecutionIssued
    );
    assert!(!launcher.script().reaped);
    launcher.script().exit = (true, Some(0), None);
    assert_eq!(
        session.wait(&owned).unwrap().phase,
        RunPhase::OutcomePending
    );
    assert!(launcher.script().reaped);
    acknowledge(&mut session);
    let next = session
        .prepare_once(2, RunOperation::InspectConfig, 2)
        .unwrap();
    acknowledge(&mut session);
    session
        .start(&next)
        .expect("旧轮已reap且空组，新轮可以启动");
}

#[test]
fn a_durable_exit_survives_reopen_even_if_the_next_backend_read_is_unknown() {
    let fixture = Fixture::new();
    let run_id = {
        let mut session = fixture.session();
        let prepared = fixture.prepared(&mut session);
        let owned = session.start(&prepared).unwrap();
        session.backend.running = false;
        session.backend.exit = ExitObservation::Code(7);
        // 主进程已退出，后代仍活：退出码先持久，尚无静止能力。
        let observed = session.wait(&owned).unwrap();
        assert_eq!(observed.exit, ExitObservation::Code(7));
        assert_eq!(observed.quiescence, Quiescence::Unconfirmed);
        owned.run_id()
    };
    let mut reopened = fixture.session();
    reopened.backend.running = false;
    reopened.backend.populated = false;
    let observed = reopened.recover(run_id).unwrap();
    assert_eq!(observed.exit, ExitObservation::Code(7));
    assert_eq!(observed.phase, RunPhase::OutcomePending);
    assert_eq!(observed.quiescence, Quiescence::Confirmed);
}
