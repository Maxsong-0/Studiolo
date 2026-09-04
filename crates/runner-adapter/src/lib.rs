//! Isolated-runner lifecycle seam and deterministic M0 Stub adapter.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use tempfile::TempDir;

/// Runner adapter health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterAvailability {
    /// Adapter can accept a run.
    Ready,
    /// Adapter is known but currently unavailable.
    Unavailable {
        /// Non-sensitive diagnostic.
        reason: String,
    },
}

/// One immutable input supplied without an ambient Vault path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInput {
    /// Logical source path.
    pub path: VaultPath,
    /// Captured source revision.
    pub revision: String,
    /// Snapshot bytes.
    pub content: Vec<u8>,
}

/// Complete immutable request for one attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSpec {
    /// Owning task.
    pub task_id: TaskId,
    /// Owning attempt.
    pub attempt_id: AttemptId,
    /// Current fencing token.
    pub fence: Fence,
    /// Human-authored instruction.
    pub instruction: String,
    /// Immutable snapshot contents.
    pub inputs: Vec<RunInput>,
}

/// Opaque run handle meaningful only to its adapter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunHandle(String);

impl RunHandle {
    /// Returns the opaque handle text for persistence.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Persistable reference used for restart isolation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRef {
    /// Adapter kind.
    pub adapter: String,
    /// Adapter-owned stable reference.
    pub opaque_id: String,
    /// Process ID for a process-backed adapter.
    pub pid: Option<u32>,
    /// OS process start identity; mandatory whenever `pid` is present.
    pub start_time_ticks: Option<u64>,
}

impl RunnerRef {
    /// Creates a non-process reference, as used by the Stub.
    #[must_use]
    pub fn opaque(adapter: impl Into<String>, opaque_id: impl Into<String>) -> Self {
        Self {
            adapter: adapter.into(),
            opaque_id: opaque_id.into(),
            pid: None,
            start_time_ticks: None,
        }
    }

    /// Creates a process reference protected against PID reuse.
    pub fn process(
        adapter: impl Into<String>,
        pid: u32,
        start_time_ticks: u64,
    ) -> Result<Self, RunnerError> {
        if pid == 0 || start_time_ticks == 0 {
            return Err(RunnerError::InvalidRunnerRef);
        }
        Ok(Self {
            adapter: adapter.into(),
            opaque_id: format!("pid:{pid}:{start_time_ticks}"),
            pid: Some(pid),
            start_time_ticks: Some(start_time_ticks),
        })
    }
}

/// Result returned once a run has been accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedRun {
    /// Adapter handle.
    pub handle: RunHandle,
    /// Durable restart reference.
    pub runner_ref: RunnerRef,
}

/// Candidate copied from runner-owned temporary storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerArtifact {
    /// Logical artifact name, not an ambient path.
    pub name: VaultPath,
    /// Candidate bytes.
    pub content: Vec<u8>,
}

/// Structured successful runner report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerReport {
    /// Owning task.
    pub task_id: TaskId,
    /// Owning attempt.
    pub attempt_id: AttemptId,
    /// Fence echoed from the immutable run spec.
    pub fence: Fence,
    /// Candidate artifacts.
    pub artifacts: Vec<RunnerArtifact>,
}

/// Current state observed from a runner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerObservation {
    /// Work remains in progress.
    Running,
    /// Work completed and returned candidates.
    Completed(RunnerReport),
    /// Capability was revoked and the run was isolated.
    Cancelled,
}

/// Runner lifecycle failures.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// A handle is unknown to this adapter instance.
    #[error("unknown runner handle")]
    UnknownHandle,
    /// The same attempt/fence was started with different immutable input.
    #[error("runner idempotency key reused with different input")]
    IdempotencyConflict,
    /// A process reference omitted its start identity.
    #[error("process runner reference requires non-zero pid and start identity")]
    InvalidRunnerRef,
    /// Adapter state mutex was poisoned.
    #[error("runner state is unavailable")]
    StateUnavailable,
    /// Temporary runner storage failed.
    #[error("runner temporary storage failed during {operation}: {source}")]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

impl RunnerError {
    fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}

/// Synchronous, object-safe lifecycle port for local and future VM runners.
pub trait RunnerAdapter: Send + Sync {
    /// Reports whether the adapter is usable.
    fn probe(&self) -> Result<AdapterAvailability, RunnerError>;

    /// Starts or idempotently recovers one immutable attempt.
    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError>;

    /// Observes state and copies candidates out of temporary storage.
    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError>;

    /// Idempotently revokes and isolates a run.
    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError>;
}

struct StubRun {
    spec: RunSpec,
    temp_dir: TempDir,
    started: StartedRun,
    cancelled: bool,
}

#[derive(Default)]
struct StubState {
    by_attempt: BTreeMap<(AttemptId, Fence), RunHandle>,
    runs: BTreeMap<RunHandle, StubRun>,
}

/// Deterministic M0 adapter that writes only to a private temporary directory.
#[derive(Default)]
pub struct StubRunner {
    state: Mutex<StubState>,
}

impl StubRunner {
    /// Creates an empty Stub adapter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn render(spec: &RunSpec) -> Vec<u8> {
        let mut output = format!(
            "# Stub result\n\nTask: {}\nAttempt: {}\nInstruction: {}\n",
            spec.task_id, spec.attempt_id, spec.instruction
        );
        for input in &spec.inputs {
            output.push_str(&format!(
                "\n## {} @ {}\n{}\n",
                input.path,
                input.revision,
                String::from_utf8_lossy(&input.content)
            ));
        }
        output.into_bytes()
    }

    fn result_path(run: &StubRun) -> PathBuf {
        run.temp_dir.path().join("result.md")
    }
}

impl RunnerAdapter for StubRunner {
    fn probe(&self) -> Result<AdapterAvailability, RunnerError> {
        Ok(AdapterAvailability::Ready)
    }

    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let key = (spec.attempt_id, spec.fence);
        if let Some(handle) = state.by_attempt.get(&key) {
            let run = state
                .runs
                .get(handle)
                .ok_or(RunnerError::StateUnavailable)?;
            if &run.spec == spec {
                return Ok(run.started.clone());
            }
            return Err(RunnerError::IdempotencyConflict);
        }

        let temp_dir = tempfile::Builder::new()
            .prefix("studiolo-stub-")
            .tempdir()
            .map_err(|error| RunnerError::io("create tempdir", error))?;
        let result_path = temp_dir.path().join("result.md");
        let mut file =
            File::create(&result_path).map_err(|error| RunnerError::io("create result", error))?;
        file.write_all(&Self::render(spec))
            .map_err(|error| RunnerError::io("write result", error))?;
        file.sync_all()
            .map_err(|error| RunnerError::io("sync result", error))?;

        let handle = RunHandle(format!("stub:{}:{}", spec.attempt_id, spec.fence.get()));
        let started = StartedRun {
            handle: handle.clone(),
            runner_ref: RunnerRef::opaque("stub", handle.as_str()),
        };
        state.by_attempt.insert(key, handle.clone());
        state.runs.insert(
            handle,
            StubRun {
                spec: spec.clone(),
                temp_dir,
                started: started.clone(),
                cancelled: false,
            },
        );
        Ok(started)
    }

    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let run = state.runs.get(handle).ok_or(RunnerError::UnknownHandle)?;
        if run.cancelled {
            return Ok(RunnerObservation::Cancelled);
        }
        let mut file = File::open(Self::result_path(run))
            .map_err(|error| RunnerError::io("open result", error))?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|error| RunnerError::io("read result", error))?;
        Ok(RunnerObservation::Completed(RunnerReport {
            task_id: run.spec.task_id,
            attempt_id: run.spec.attempt_id,
            fence: run.spec.fence,
            artifacts: vec![RunnerArtifact {
                name: VaultPath::parse("result.md").expect("static path is valid"),
                content,
            }],
        }))
    }

    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        let run = state
            .runs
            .get_mut(handle)
            .ok_or(RunnerError::UnknownHandle)?;
        run.cancelled = true;
        Ok(())
    }
}
