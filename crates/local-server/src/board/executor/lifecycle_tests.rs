use super::launch_failure;
use studiolo_runner_adapter::{LaunchStage, RunnerError};

#[test]
fn only_explicit_pre_tool_launch_failure_can_enter_retry_policy() {
    let handshake = RunnerError::LaunchFailed {
        stage: LaunchStage::Handshake,
    };
    assert!(launch_failure(&handshake, false, Some(0), Some(0)));
    for (stopping, observed, persisted) in [
        (true, Some(0), Some(0)),
        (false, Some(1), Some(0)),
        (false, Some(0), Some(1)),
        (false, None, Some(0)),
        (false, Some(0), None),
    ] {
        assert!(!launch_failure(&handshake, stopping, observed, persisted));
    }
    for failure in [
        RunnerError::Protocol,
        RunnerError::BudgetExceeded,
        RunnerError::Unavailable,
    ] {
        assert!(!launch_failure(&failure, false, Some(0), Some(0)));
    }
}
