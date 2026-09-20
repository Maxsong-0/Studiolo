//! Process lifecycle shared by Claude Code and Codex; disabled until live isolation passes.
use crate::{
    AdapterAvailability, BudgetPhase, CliProvider, LaunchStage, LinuxIsolation, RunHandle, RunSpec,
    RunnerAdapter, RunnerArtifact, RunnerError, RunnerObservation, RunnerProgress, RunnerRef,
    RunnerReport, StartedRun, snapshot::Snapshot,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Child, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use studiolo_core::VaultPath;

/// Host-owned runner deployment. Never constructed from an untrusted HTTP body.
#[derive(Clone, Debug)]
pub struct RunnerLaunchConfig {
    /// Selected pinned CLI adapter.
    pub provider: CliProvider,
    /// SHA-256 of the pinned native CLI binary installed in the immutable runtime root.
    pub cli_sha256: String,
    /// Verified Linux namespace/cgroup profile.
    pub isolation: LinuxIsolation,
    /// Trusted source checkout, containing no canonical Vault data or secrets.
    pub source_checkout: PathBuf,
    /// Scratch must be mounted on a bounded filesystem (at most 512 MiB).
    pub attempt_root: PathBuf,
    /// Durable directory outside the scratch area for all unlanded patches.
    pub retained_root: PathBuf,
    /// Private short-lived Gateway capability file issued for exactly this attempt.
    pub capability_file: PathBuf,
}
struct Run {
    spec: RunSpec,
    started: StartedRun,
    snapshot: Snapshot,
    child: Child,
    unit: String,
    began: Instant,
    cancelled: Option<Instant>,
    progress: RunnerProgress,
    observed: Arc<Mutex<RunnerProgress>>,
    stopped_receipt: Arc<Mutex<Option<serde_json::Value>>>,
    output_done: mpsc::Receiver<()>,
    output_joined: bool,
    cancel_marker: PathBuf,
    failed: bool,
    report: Option<RunnerReport>,
    retained: PathBuf,
}
/// Real CLI adapter; long-lived credentials are never accepted by this type.
pub struct RealRunner {
    config: RunnerLaunchConfig,
    runs: Mutex<BTreeMap<RunHandle, Run>>,
    last_probe: Mutex<Option<crate::AdapterProbe>>,
}
impl RealRunner {
    /// Creates an adapter in the unavailable-by-default state until the live probe succeeds.
    #[must_use]
    pub fn new(config: RunnerLaunchConfig) -> Self {
        Self {
            config,
            runs: Mutex::new(BTreeMap::new()),
            last_probe: Mutex::new(None),
        }
    }
    fn launch(&self, spec: &RunSpec) -> Result<Run, RunnerError> {
        let execution = spec
            .execution
            .as_ref()
            .ok_or(RunnerError::InvalidExecution)?;
        execution.validate(spec, self.config.provider)?;
        if !matches!(self.probe()?, AdapterAvailability::Ready) {
            return Err(RunnerError::Unavailable);
        }
        let name = format!(
            "{}-{}-{}",
            self.config.provider.name(),
            spec.attempt_id,
            spec.fence.get()
        );
        let scratch = self.config.attempt_root.join(&name);
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir(&scratch).map_err(|e| RunnerError::io("attempt directory", e))?;
        fs::set_permissions(&scratch, fs::Permissions::from_mode(0o700))
            .map_err(|e| RunnerError::io("attempt permissions", e))?;
        let snapshot = Snapshot::create(&self.config.source_checkout, &scratch)?;
        if snapshot.base != execution.repository_tree {
            return Err(RunnerError::InvalidExecution);
        }
        let inputs = scratch.join("inputs");
        fs::create_dir(&inputs).map_err(|e| RunnerError::io("input directory", e))?;
        // The authenticated Gateway resolves this job identity to an independently stored grant.
        // No bearer or provider credential is serialized in a RunSpec or persisted here.
        let job = serde_json::json!({"task_id": spec.task_id, "attempt_id": spec.attempt_id,
            "fence": spec.fence, "provider": self.config.provider, "model": execution.model,
            "args": self.config.provider.args(&execution.model), "instruction": spec.instruction,
            "budget": spec.budget});
        fs::write(
            inputs.join("job.json"),
            serde_json::to_vec(&job).map_err(|_| RunnerError::InvalidExecution)?,
        )
        .map_err(|e| RunnerError::io("job manifest", e))?;
        for (index, input) in spec.inputs.iter().enumerate() {
            fs::write(inputs.join(format!("input-{index}")), &input.content)
                .map_err(|e| RunnerError::io("input snapshot", e))?;
        }
        let unit = format!(
            "studiolo-runner-{}-{}.scope",
            spec.attempt_id,
            spec.fence.get()
        );
        let mut command =
            self.config
                .isolation
                .command(&unit, &snapshot.work, &inputs, spec.budget.max_seconds);
        command
            .args(["--ro-bind"])
            .arg(&self.config.capability_file)
            .arg("/run/job-capability")
            .args(["/usr/bin/python3", "/usr/lib/studiolo/launch.py"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| RunnerError::LaunchFailed {
            stage: LaunchStage::Isolation,
        })?;
        let observed = Arc::new(Mutex::new(RunnerProgress::default()));
        let progress_sink = observed.clone();
        let stopped_receipt = Arc::new(Mutex::new(None));
        let receipt_sink = stopped_receipt.clone();
        let (output_finished, output_done) = mpsc::channel();
        let output = child.stdout.take().ok_or(RunnerError::Protocol)?;
        let isolation = self.config.isolation.clone();
        let monitored_unit = unit.clone();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Read};
            let mut reader = BufReader::new(output).take(1024 * 1024 + 1);
            let mut line = String::new();
            while let Ok(size) = reader.read_line(&mut line) {
                if size == 0 {
                    break;
                }
                if let Ok(progress) = serde_json::from_str::<RunnerProgress>(&line)
                    && let Ok(mut state) = progress_sink.lock()
                {
                    state.tool_calls = state.tool_calls.max(progress.tool_calls);
                    state.edits = state.edits.max(progress.edits);
                    state.phase = progress.phase;
                }
                if line.len() <= 131_072
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                    && let Some(receipt) = value.get("stopped_receipt")
                    && receipt.get("status").and_then(|v| v.as_str()) == Some("stopped")
                    && let Ok(mut saved) = receipt_sink.lock()
                {
                    *saved = Some(receipt.clone());
                }
                line.clear();
            }
            if reader.limit() == 0 {
                let _ = isolation.stop_unit(&monitored_unit, "SIGKILL");
            }
            let _ = output_finished.send(());
        });
        let handle = RunHandle(format!(
            "{}:{}:{}",
            self.config.provider.name(),
            spec.attempt_id,
            spec.fence.get()
        ));
        let started = StartedRun {
            handle,
            runner_ref: RunnerRef::opaque(self.config.provider.name(), &unit),
        };
        Ok(Run {
            spec: spec.clone(),
            started,
            snapshot,
            child,
            unit,
            began: Instant::now(),
            cancelled: None,
            progress: RunnerProgress::default(),
            observed,
            stopped_receipt,
            output_done,
            output_joined: false,
            cancel_marker: inputs.join("cancel-requested"),
            failed: false,
            report: None,
            retained: self
                .config
                .retained_root
                .join(format!("{name}-unlanded.patch")),
        })
    }
    fn cancelled_observation(run: &Run) -> Result<RunnerObservation, RunnerError> {
        let receipt = run
            .stopped_receipt
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?
            .clone();
        match (receipt, &run.report) {
            (Some(receipt), Some(report)) => {
                use std::{io::Write, os::unix::fs::OpenOptionsExt};
                let bytes = serde_json::to_vec(&receipt).map_err(|_| RunnerError::Protocol)?;
                let path = run.retained.with_extension("stopped.json");
                match fs::read(&path) {
                    Ok(existing) if existing == bytes => {}
                    Ok(_) => return Err(RunnerError::IdempotencyConflict),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(&path)
                            .map_err(|e| RunnerError::io("stopped receipt retention", e))?;
                        file.write_all(&bytes)
                            .and_then(|()| file.sync_all())
                            .map_err(|e| RunnerError::io("stopped receipt retention", e))?;
                        fs::File::open(path.parent().ok_or(RunnerError::InvalidExecution)?)
                            .and_then(|f| f.sync_all())
                            .map_err(|e| RunnerError::io("stopped receipt directory", e))?;
                    }
                    Err(error) => return Err(RunnerError::io("stopped receipt retention", error)),
                }
                let mut report = report.clone();
                report.artifacts.push(RunnerArtifact {
                    name: VaultPath::parse("stopped.json").map_err(|_| RunnerError::Protocol)?,
                    content: serde_json::to_vec(&receipt).map_err(|_| RunnerError::Protocol)?,
                });
                Ok(RunnerObservation::Stopped { report, receipt })
            }
            _ => Ok(RunnerObservation::Cancelled),
        }
    }
    fn request_cancel(run: &mut Run) -> Result<(), RunnerError> {
        if run.cancelled.is_none() {
            fs::write(&run.cancel_marker, b"cancel_requested")
                .map_err(|e| RunnerError::io("cancel notice", e))?;
            run.cancelled = Some(Instant::now());
        }
        Ok(())
    }
    fn retain(run: &mut Run) -> Result<(), RunnerError> {
        if run.report.is_some() {
            return Ok(());
        }
        let (metadata, patch) = run.snapshot.preserve(&run.retained)?;
        let meta = serde_json::to_vec(&metadata).map_err(|_| RunnerError::Protocol)?;
        run.report = Some(RunnerReport {
            task_id: run.spec.task_id,
            attempt_id: run.spec.attempt_id,
            fence: run.spec.fence,
            artifacts: vec![
                RunnerArtifact {
                    name: VaultPath::parse("candidate.patch").map_err(|_| RunnerError::Protocol)?,
                    content: patch,
                },
                RunnerArtifact {
                    name: VaultPath::parse("patch.json").map_err(|_| RunnerError::Protocol)?,
                    content: meta,
                },
            ],
        });
        Ok(())
    }
}
impl RunnerAdapter for RealRunner {
    fn probe(&self) -> Result<AdapterAvailability, RunnerError> {
        if !self.config.attempt_root.is_dir() || !self.config.retained_root.is_dir() {
            return Ok(AdapterAvailability::Unavailable {
                reason: "runner private scratch or durable candidate directory missing".into(),
            });
        }
        let launcher = self
            .config
            .isolation
            .runtime_root
            .join("usr/lib/studiolo/launch.py");
        if fs::read(launcher).ok().as_deref()
            != Some(include_bytes!("../../../deploy/runner/launch.py").as_slice())
        {
            return Ok(AdapterAvailability::Unavailable {
                reason: "pinned runner launcher missing or changed".into(),
            });
        }
        use sha2::{Digest, Sha256};
        let binary = self
            .config
            .isolation
            .runtime_root
            .join("usr/local/bin")
            .join(self.config.provider.name());
        let Ok(mut binary) = fs::File::open(binary) else {
            return Ok(AdapterAvailability::Unavailable {
                reason: "pinned CLI binary unavailable".into(),
            });
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 65536];
        loop {
            use std::io::Read;
            let size = binary
                .read(&mut buffer)
                .map_err(|e| RunnerError::io("CLI digest", e))?;
            if size == 0 {
                break;
            }
            hasher.update(&buffer[..size]);
        }
        let digest: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if digest != self.config.cli_sha256 || !matches!(self.config.cli_sha256.len(), 64) {
            return Ok(AdapterAvailability::Unavailable {
                reason: "pinned CLI binary digest mismatch".into(),
            });
        }
        let report = self
            .config
            .isolation
            .probe_cli_at(&self.config.attempt_root, self.config.provider)?;
        let availability = report.availability.clone();
        *self
            .last_probe
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)? = Some(report);
        Ok(availability)
    }
    fn probe_report(&self) -> Result<crate::AdapterProbe, RunnerError> {
        *self
            .last_probe
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)? = None;
        let availability = self.probe()?;
        if let Some(report) = self
            .last_probe
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?
            .clone()
        {
            return Ok(report);
        }
        let missing = match &availability {
            AdapterAvailability::Unavailable { reason } => vec![reason.clone()],
            _ => Vec::new(),
        };
        Ok(crate::AdapterProbe {
            availability,
            binary_version: None,
            protocol_capabilities: Vec::new(),
            missing_prerequisites: missing,
        })
    }
    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let handle = RunHandle(format!(
            "{}:{}:{}",
            self.config.provider.name(),
            spec.attempt_id,
            spec.fence.get()
        ));
        if let Some(run) = runs.get(&handle) {
            return if &run.spec == spec {
                Ok(run.started.clone())
            } else {
                Err(RunnerError::IdempotencyConflict)
            };
        }
        let run = self.launch(spec).map_err(|error| match error {
            RunnerError::Io { .. } => RunnerError::LaunchFailed {
                stage: LaunchStage::Snapshot,
            },
            other => other,
        })?;
        let started = run.started.clone();
        runs.insert(handle, run);
        Ok(started)
    }
    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let run = runs.get_mut(handle).ok_or(RunnerError::UnknownHandle)?;
        if run.failed {
            return Err(RunnerError::Protocol);
        }
        if let Some(report) = &run.report {
            return if run.cancelled.is_some() {
                Self::cancelled_observation(run)
            } else {
                Ok(RunnerObservation::Completed(report.clone()))
            };
        }
        // Counters are advisory; no runner-controlled host filesystem path is opened.
        if let Ok(progress) = run.observed.lock() {
            run.progress.tool_calls = run.progress.tool_calls.max(progress.tool_calls);
            run.progress.edits = run.progress.edits.max(progress.edits);
        }
        run.progress.phase = run
            .spec
            .budget
            .phase(run.progress.tool_calls, run.began.elapsed().as_secs());
        if run.cancelled.is_none() && run.progress.phase == BudgetPhase::Exhausted {
            Self::request_cancel(run)?;
        }
        if run
            .cancelled
            .is_some_and(|at| at.elapsed().as_secs() >= 180)
        {
            self.config.isolation.stop_unit(&run.unit, "SIGKILL")?;
        }
        let Some(status) = run
            .child
            .try_wait()
            .map_err(|e| RunnerError::io("observe process", e))?
        else {
            return Ok(RunnerObservation::Running);
        };
        if !run.output_joined {
            run.output_done
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| RunnerError::Protocol)?;
            run.output_joined = true;
            let final_progress = *run
                .observed
                .lock()
                .map_err(|_| RunnerError::StateUnavailable)?;
            run.progress.tool_calls = run.progress.tool_calls.max(final_progress.tool_calls);
            run.progress.edits = run.progress.edits.max(final_progress.edits);
        }
        Self::retain(run)?;
        if run.cancelled.is_some() {
            return Self::cancelled_observation(run);
        }
        if !status.success() {
            run.failed = true;
            return Err(
                if run.progress.tool_calls == 0 && !matches!(status.code(), Some(70..=72)) {
                    RunnerError::LaunchFailed {
                        stage: LaunchStage::Handshake,
                    }
                } else {
                    RunnerError::Protocol
                },
            );
        }
        Ok(RunnerObservation::Completed(
            run.report.clone().ok_or(RunnerError::Protocol)?,
        ))
    }
    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let run = runs.get_mut(handle).ok_or(RunnerError::UnknownHandle)?;
        if run.cancelled.is_none() {
            Self::request_cancel(run)?;
        }
        Ok(())
    }
    fn progress(&self, handle: &RunHandle) -> Result<RunnerProgress, RunnerError> {
        self.runs
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?
            .get(handle)
            .map(|run| run.progress)
            .ok_or(RunnerError::UnknownHandle)
    }
}

#[cfg(test)]
#[path = "real_tests.rs"]
mod tests;
