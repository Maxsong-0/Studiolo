//! M0 `runner` 接缝与 `Stub` 适配器的契约测试。

use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_runner_adapter::{
    AdapterAvailability, RunInput, RunSpec, RunnerAdapter, RunnerError, RunnerObservation,
    RunnerRef, StubRunner,
};

fn spec() -> RunSpec {
    RunSpec {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        instruction: "summarize the supplied note".to_owned(),
        budget: Default::default(),
        execution: None,
        inputs: vec![RunInput {
            path: VaultPath::parse("notes/input.md").unwrap(),
            revision: "revision-1".to_owned(),
            content: b"hello".to_vec(),
        }],
    }
}

fn accepts_trait_object(_runner: &dyn RunnerAdapter) {}

#[test]
fn runner_interface_is_object_safe_and_probe_is_explicit() {
    let runner = StubRunner::new();
    accepts_trait_object(&runner);
    assert_eq!(runner.probe().unwrap(), AdapterAvailability::Ready);
}

#[test]
fn stub_writes_temporary_output_and_observe_returns_candidate_bytes() {
    let runner = StubRunner::new();
    let spec = spec();
    let started = runner.start(&spec).expect("start stub");

    let RunnerObservation::Completed(report) = runner.observe(&started.handle).unwrap() else {
        panic!("stub should complete synchronously");
    };
    assert_eq!(report.task_id, spec.task_id);
    assert_eq!(report.attempt_id, spec.attempt_id);
    assert_eq!(report.fence, spec.fence);
    assert_eq!(report.artifacts.len(), 1);
    assert_eq!(report.artifacts[0].name.as_str(), "result.md");
    assert!(String::from_utf8_lossy(&report.artifacts[0].content).contains("hello"));
}

#[test]
fn start_is_idempotent_for_the_same_attempt_and_fence() {
    let runner = StubRunner::new();
    let spec = spec();
    let first = runner.start(&spec).unwrap();
    let replay = runner.start(&spec).unwrap();
    assert_eq!(first, replay);

    let mut changed = spec;
    changed.instruction.push_str(" changed");
    assert!(matches!(
        runner.start(&changed),
        Err(RunnerError::IdempotencyConflict)
    ));
}

#[test]
fn cancel_is_idempotent_and_observe_reports_cancelled() {
    let runner = StubRunner::new();
    let started = runner.start(&spec()).unwrap();
    runner.cancel(&started.handle).unwrap();
    runner.cancel(&started.handle).unwrap();
    assert_eq!(
        runner.observe(&started.handle).unwrap(),
        RunnerObservation::Cancelled
    );
}

#[test]
fn process_reference_requires_start_identity_to_prevent_pid_reuse() {
    assert!(RunnerRef::process("local", 42, 0).is_err());
    let reference = RunnerRef::process("local", 42, 99).expect("valid process ref");
    assert_eq!(reference.pid, Some(42));
    assert_eq!(reference.start_time_ticks, Some(99));
}
