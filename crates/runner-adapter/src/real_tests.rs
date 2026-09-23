//! 聚焦的私有生命周期夹具：不涉及供应商 CLI、namespace 服务或模型请求。
use super::*;
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};
use studiolo_core::{AttemptId, Fence, TaskId};

struct Fixture {
    _temp: tempfile::TempDir,
    runner: RealRunner,
    handle: RunHandle,
    done: Option<mpsc::Sender<()>>,
}

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Fixture {
    fn new(exit: u8) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source");
        let attempts = root.join("attempts");
        let attempt = attempts.join("attempt");
        let retained = root.join("retained");
        for path in [&source, &attempts, &attempt, &retained] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        git(&source, &["init", "-q"]);
        fs::write(source.join("file.txt"), "baseline\n").unwrap();
        git(&source, &["add", "."]);
        git(
            &source,
            &["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"],
        );
        let snapshot = Snapshot::create(&source, &attempt).unwrap();
        fs::write(snapshot.work.join("file.txt"), "candidate\n").unwrap();
        let inputs = attempt.join("inputs");
        fs::create_dir(&inputs).unwrap();
        let spec = RunSpec {
            task_id: TaskId::new(),
            attempt_id: AttemptId::new(),
            fence: Fence::new(1).unwrap(),
            instruction: "fixture".into(),
            inputs: vec![],
            budget: crate::RunBudget::default(),
            execution: None,
        };
        let handle = RunHandle("fixture".into());
        let mut child = Command::new("/bin/sh")
            .env_clear()
            .args(["-c", &format!("exit {exit}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let (done, output_done) = mpsc::channel();
        let run = Run {
            spec,
            started: StartedRun {
                handle: handle.clone(),
                runner_ref: RunnerRef::opaque("codex", "fixture.scope"),
            },
            snapshot,
            child,
            unit: "fixture.scope".into(),
            began: Instant::now(),
            cancelled: None,
            progress: RunnerProgress::default(),
            observed: Arc::new(Mutex::new(RunnerProgress::default())),
            stopped_receipt: Arc::new(Mutex::new(None)),
            output_done,
            output_joined: false,
            cancel_marker: inputs.join("cancel-requested"),
            failed: false,
            report: None,
            retained: retained.join("candidate.patch"),
        };
        let runner = RealRunner::new(RunnerLaunchConfig {
            provider: CliProvider::Codex,
            cli_sha256: "0".repeat(64),
            isolation: LinuxIsolation {
                runtime_root: root.join("missing-runtime"),
                bubblewrap: root.join("missing-bwrap"),
                systemd_run: root.join("missing-systemd"),
                gateway_socket: root.join("missing-socket"),
            },
            source_checkout: source,
            attempt_root: attempts,
            retained_root: retained,
            capability_file: root.join("unused-capability"),
        });
        runner.runs.lock().unwrap().insert(handle.clone(), run);
        Self {
            _temp: temp,
            runner,
            handle,
            done: Some(done),
        }
    }

    fn drain(&mut self) {
        self.done.take().unwrap().send(()).unwrap();
    }
}

#[test]
fn exit_waits_for_final_drain_before_classifying_zero_tools() {
    let mut fixture = Fixture::new(7);
    let observed = fixture.runner.runs.lock().unwrap()[&fixture.handle]
        .observed
        .clone();
    let done = fixture.done.take().unwrap();
    let monitor = std::thread::spawn(move || {
        // 子进程已退出；它独立的输出读取线程稍后才发布最后一个事件。
        std::thread::sleep(Duration::from_millis(50));
        *observed.lock().unwrap() = RunnerProgress {
            tool_calls: 1,
            edits: 1,
            phase: BudgetPhase::Working,
        };
        done.send(()).unwrap();
    });
    let result = fixture.runner.observe(&fixture.handle);
    monitor.join().unwrap();
    assert!(matches!(result, Err(RunnerError::Protocol)));
    assert_eq!(
        fixture.runner.progress(&fixture.handle).unwrap().tool_calls,
        1
    );
    let runs = fixture.runner.runs.lock().unwrap();
    assert!(runs[&fixture.handle].retained.is_file());
    assert!(runs[&fixture.handle].report.is_some());
}

#[test]
fn ordinary_failure_before_any_tool_is_typed_handshake_failure() {
    let mut fixture = Fixture::new(7);
    fixture.drain();
    assert!(matches!(
        fixture.runner.observe(&fixture.handle),
        Err(RunnerError::LaunchFailed {
            stage: LaunchStage::Handshake
        })
    ));
}

#[test]
fn launcher_protocol_exit_codes_are_never_retryable_handshake_failures() {
    for code in [70, 71, 72] {
        let mut fixture = Fixture::new(code);
        fixture.drain();
        assert!(matches!(
            fixture.runner.observe(&fixture.handle),
            Err(RunnerError::Protocol)
        ));
    }
}

#[test]
fn missing_drain_confirmation_cannot_publish_completion() {
    let mut fixture = Fixture::new(0);
    drop(fixture.done.take());
    assert!(matches!(
        fixture.runner.observe(&fixture.handle),
        Err(RunnerError::Protocol)
    ));
    assert!(
        fixture.runner.runs.lock().unwrap()[&fixture.handle]
            .report
            .is_none()
    );
}

#[test]
fn cancellation_is_idempotent_and_never_infers_a_stopped_receipt() {
    let mut fixture = Fixture::new(7);
    fixture.runner.cancel(&fixture.handle).unwrap();
    let at = fixture.runner.runs.lock().unwrap()[&fixture.handle].cancelled;
    fixture.runner.cancel(&fixture.handle).unwrap();
    assert_eq!(
        at,
        fixture.runner.runs.lock().unwrap()[&fixture.handle].cancelled
    );
    fixture.drain();
    assert!(matches!(
        fixture.runner.observe(&fixture.handle).unwrap(),
        RunnerObservation::Cancelled
    ));
    let runs = fixture.runner.runs.lock().unwrap();
    assert_eq!(
        fs::read(&runs[&fixture.handle].cancel_marker).unwrap(),
        b"cancel_requested"
    );
    assert!(runs[&fixture.handle].retained.is_file());
    assert!(
        !runs[&fixture.handle]
            .retained
            .with_extension("stopped.json")
            .exists()
    );
}

#[test]
fn explicit_stopped_receipt_is_retained_and_replayed_without_owner_acceptance() {
    let mut fixture = Fixture::new(7);
    let receipt = serde_json::json!({"status":"stopped", "remaining":["unfinished"],
        "acceptance":[{"criterion":"fixture", "result":"unverified", "evidence":[]} ]});
    {
        let runs = fixture.runner.runs.lock().unwrap();
        *runs[&fixture.handle].stopped_receipt.lock().unwrap() = Some(receipt.clone());
    }
    fixture.runner.cancel(&fixture.handle).unwrap();
    fixture.drain();
    let first = fixture.runner.observe(&fixture.handle).unwrap();
    let RunnerObservation::Stopped {
        report,
        receipt: received,
    } = &first
    else {
        panic!("explicit cancelled receipt must remain untrusted Stopped");
    };
    assert_eq!(received, &receipt);
    assert_eq!(report.artifacts.len(), 3);
    assert_eq!(report.artifacts[2].name.as_str(), "stopped.json");
    assert_eq!(fixture.runner.observe(&fixture.handle).unwrap(), first);
    let runs = fixture.runner.runs.lock().unwrap();
    let path = runs[&fixture.handle]
        .retained
        .with_extension("stopped.json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap(),
        receipt
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn budget_expiry_cancels_even_when_child_exits_without_a_tool() {
    let mut fixture = Fixture::new(7);
    fixture
        .runner
        .runs
        .lock()
        .unwrap()
        .get_mut(&fixture.handle)
        .unwrap()
        .began = Instant::now() - Duration::from_secs(2101);
    fixture.drain();
    assert!(matches!(
        fixture.runner.observe(&fixture.handle).unwrap(),
        RunnerObservation::Cancelled
    ));
    assert_eq!(
        fixture.runner.progress(&fixture.handle).unwrap().phase,
        BudgetPhase::Exhausted
    );
}

#[test]
fn failed_current_probe_discards_previous_ready_diagnostics() {
    let fixture = Fixture::new(0);
    *fixture.runner.last_probe.lock().unwrap() = Some(crate::AdapterProbe {
        availability: AdapterAvailability::Ready,
        binary_version: Some("stale-version".into()),
        protocol_capabilities: vec!["stale-capability".into()],
        missing_prerequisites: vec![],
    });
    let report = fixture.runner.probe_report().unwrap();
    assert!(matches!(
        report.availability,
        AdapterAvailability::Unavailable { .. }
    ));
    assert!(report.binary_version.is_none());
    assert!(report.protocol_capabilities.is_empty());
    assert!(!report.missing_prerequisites.is_empty());
}
