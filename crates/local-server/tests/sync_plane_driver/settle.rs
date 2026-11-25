//! 收旧槽的契约（T-239）：停在 `Prepared` 的旧运行从没发过启动请求，收成「没执行」再补审计；
//! 发过启动请求的一律只观察，证明不了闭合就停在 `Transport`。

use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::headless::{RunSlot, settle_active};
use studiolo_sync_transport::owned_run::{RunId, RunPhase};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Flush,
    Recover,
    Cancel,
}

struct FakeSlot {
    /// 活动运行所处阶段；`None` 是空槽。
    phase: Option<RunPhase>,
    /// 第几次补审计会失败（从 1 数）。
    failing_flush: Option<usize>,
    calls: Vec<Call>,
}

impl FakeSlot {
    fn at(phase: Option<RunPhase>) -> Self {
        Self {
            phase,
            failing_flush: None,
            calls: Vec::new(),
        }
    }

    fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|made| **made == call).count()
    }
}

impl RunSlot for FakeSlot {
    fn flush_audit(&mut self) -> Result<(), PlaneError> {
        self.calls.push(Call::Flush);
        if self.failing_flush == Some(self.count(Call::Flush)) {
            return Err(PlaneError::Audit);
        }
        // 只有 Outcome 已固定的运行，补完审计才退出活动槽。
        if matches!(
            self.phase,
            Some(RunPhase::Cancelled | RunPhase::OutcomePending)
        ) {
            self.phase = None;
        }
        Ok(())
    }

    fn active(&self) -> Result<Option<RunId>, PlaneError> {
        Ok(self.phase.map(|_| RunId::from_bytes([3; 16])))
    }

    fn recover(&mut self, _: RunId) -> Result<RunPhase, PlaneError> {
        self.calls.push(Call::Recover);
        if self.phase == Some(RunPhase::Quiesced) {
            self.phase = Some(RunPhase::OutcomePending);
        }
        self.phase.ok_or(PlaneError::Transport)
    }

    fn cancel_unlaunched(&mut self, _: RunId) -> Result<RunPhase, PlaneError> {
        self.calls.push(Call::Cancel);
        if self.phase != Some(RunPhase::Prepared) {
            return Err(PlaneError::Transport);
        }
        self.phase = Some(RunPhase::Cancelled);
        Ok(RunPhase::Cancelled)
    }
}

#[test]
fn an_empty_slot_only_flushes_the_audit() {
    let mut slot = FakeSlot::at(None);
    assert_eq!(settle_active(&mut slot), Ok(()));
    assert_eq!(slot.calls, vec![Call::Flush]);
}

#[test]
fn a_prepared_run_is_cancelled_once_and_its_outcome_is_flushed() {
    let mut slot = FakeSlot::at(Some(RunPhase::Prepared));
    assert_eq!(settle_active(&mut slot), Ok(()));
    assert_eq!(
        slot.calls,
        vec![Call::Flush, Call::Recover, Call::Cancel, Call::Flush]
    );
    assert_eq!(slot.phase, None);
}

#[test]
fn an_audit_failure_while_closing_stops_this_round_and_the_next_round_finishes() {
    // 第 1 次（补意图）失败：还没取消。第 2 次（补结果）失败：已取消，下一轮补完。
    for (failing, cancels) in [(1, 0), (2, 1)] {
        let mut slot = FakeSlot::at(Some(RunPhase::Prepared));
        slot.failing_flush = Some(failing);
        assert_eq!(settle_active(&mut slot), Err(PlaneError::Audit));
        assert_eq!(slot.count(Call::Cancel), cancels);
        slot.failing_flush = None;
        slot.calls.clear();
        assert_eq!(settle_active(&mut slot), Ok(()));
        assert_eq!(slot.phase, None);
        assert_eq!(slot.count(Call::Cancel), 1 - cancels);
    }
}

#[test]
fn a_run_that_may_have_been_launched_is_only_observed() {
    for phase in [
        RunPhase::LaunchIssued,
        RunPhase::Contained,
        RunPhase::ExecutionIssued,
        RunPhase::Running,
        RunPhase::StopRequested,
    ] {
        let mut slot = FakeSlot::at(Some(phase));
        assert_eq!(settle_active(&mut slot), Err(PlaneError::Transport));
        assert_eq!(slot.count(Call::Cancel), 0, "{phase:?}");
        assert_eq!(slot.phase, Some(phase));
    }
}

#[test]
fn a_quiesced_run_still_closes_through_its_fixed_outcome() {
    let mut slot = FakeSlot::at(Some(RunPhase::Quiesced));
    assert_eq!(settle_active(&mut slot), Ok(()));
    assert_eq!(slot.calls, vec![Call::Flush, Call::Recover, Call::Flush]);
}
