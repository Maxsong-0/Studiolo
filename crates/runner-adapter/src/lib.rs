//! 隔离 runner 的生命周期接缝与确定性 M0 Stub；另含真实 CLI 适配器、Linux 隔离、快照与落地。

mod budget;
mod cli;
mod isolation;
mod landing;
mod real;
mod snapshot;

pub use budget::{BudgetPhase, RunBudget, RunnerProgress};
pub use cli::{CliProvider, ExecutionSpec, SourceEgress};
pub use isolation::LinuxIsolation;
pub use landing::{CodeLandingPlan, CodeLandingResult, advance_code_ref, apply_code_patch};
pub use real::{RealRunner, RunnerLaunchConfig};
pub use snapshot::{
    CodeSnapshot, PatchMetadata, SnapshotCleanupReport, SnapshotCleanupRequest,
    cleanup_orphan_snapshots,
};

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

/// runner 适配器的健康状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterAvailability {
    /// 适配器可以接受运行。
    Ready,
    /// 适配器已知，但当前不可用。
    Unavailable {
        /// 不含敏感信息的诊断说明。
        reason: String,
    },
}

/// 实时诊断回执；只要有前提不可用，就绝不表示适配器可用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterProbe {
    /// 本次探测后的实际可用性。
    pub availability: AdapterAvailability,
    /// 在隔离环境内从版本钉死的二进制读取的版本号。
    pub binary_version: Option<String>,
    /// 已对照所装二进制的 help 输出验证过的协议参数。
    pub protocol_capabilities: Vec<String>,
    /// 具体缺失或失败的边界；绝不探查个人登录信息。
    pub missing_prerequisites: Vec<String>,
}

/// 一份不可变输入；只提供内容，不提供可直接访问的 Vault 文件路径。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInput {
    /// 逻辑来源路径。
    pub path: VaultPath,
    /// 捕获时的来源修订版本。
    pub revision: String,
    /// 快照字节。
    pub content: Vec<u8>,
}

/// 一次 attempt 的完整不可变请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSpec {
    /// 所属任务。
    pub task_id: TaskId,
    /// 所属 attempt。
    pub attempt_id: AttemptId,
    /// 当前的 fencing token。
    pub fence: Fence,
    /// 人工撰写的指令。
    pub instruction: String,
    /// 不可变的快照内容。
    pub inputs: Vec<RunInput>,
    /// 资源与工作量预算；缺省值保证旧的已持久化 spec 仍可读取。
    #[serde(default)]
    pub budget: RunBudget,
    /// 云端执行绑定；缺失时真实适配器必须拒绝。
    #[serde(default)]
    pub execution: Option<ExecutionSpec>,
}

/// 不透明的运行句柄，只对其所属适配器有意义。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunHandle(String);

impl RunHandle {
    /// 返回不透明句柄的文本，用于持久化。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 可持久化的引用，供重启后隔离遗留运行使用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRef {
    /// 适配器类型。
    pub adapter: String,
    /// 适配器自有的稳定引用。
    pub opaque_id: String,
    /// 进程型适配器的进程 ID。
    pub pid: Option<u32>,
    /// 操作系统层面的进程启动身份；只要有 `pid` 就必须提供。
    pub start_time_ticks: Option<u64>,
}

impl RunnerRef {
    /// 创建非进程型引用（Stub 使用此种）。
    #[must_use]
    pub fn opaque(adapter: impl Into<String>, opaque_id: impl Into<String>) -> Self {
        Self {
            adapter: adapter.into(),
            opaque_id: opaque_id.into(),
            pid: None,
            start_time_ticks: None,
        }
    }

    /// 创建能防御 PID 复用的进程引用。
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

/// 运行被接受后返回的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedRun {
    /// 适配器句柄。
    pub handle: RunHandle,
    /// 持久化的重启引用。
    pub runner_ref: RunnerRef,
}

/// 从 runner 自有临时存储中复制出的候选产物。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerArtifact {
    /// 逻辑产物名，而不是可直接访问的文件系统路径。
    pub name: VaultPath,
    /// 候选字节。
    pub content: Vec<u8>,
}

/// 结构化的 runner 成功报告。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerReport {
    /// 所属任务。
    pub task_id: TaskId,
    /// 所属 attempt。
    pub attempt_id: AttemptId,
    /// 从不可变运行 spec 回显的 fence。
    pub fence: Fence,
    /// 候选产物。
    pub artifacts: Vec<RunnerArtifact>,
}

/// 从 runner 观测到的当前状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerObservation {
    /// 工作仍在进行。
    Running,
    /// 工作已完成并返回候选产物。
    Completed(RunnerReport),
    /// 被取消的 CLI 提交了显式但不可信的停止回执，以及已保留的候选产物。
    /// 可信引擎必须先校验回执与取消期限，才能接受它。
    Stopped {
        /// 实际保留的补丁与元数据，含原始的停止回执 JSON。
        report: RunnerReport,
        /// CLI 按协议原样提供的回执；绝不视为所有者评估已通过。
        receipt: serde_json::Value,
    },
    /// capability 已被撤销；尚未收到有效的停止回执。
    Cancelled,
}

/// 执行开始之前的稳定失败点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchStage {
    /// 无法准备输入快照。
    Snapshot,
    /// 无法启动隔离服务。
    Isolation,
    /// CLI 未能产生有效的协议事件。
    Handshake,
}

/// runner 生命周期中的失败。
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// 隔离或部署前提不可用：真实适配器未通过验证、平台不支持，或无法停止隔离单元。
    #[error("runner isolation is unavailable")]
    Unavailable,
    /// 首次工具调用之前的失败，可按 launch_failure 重试。
    #[error("runner launch failed at {stage:?}")]
    LaunchFailed {
        /// 稳定、不含敏感信息的失败边界。
        stage: LaunchStage,
    },
    /// 不可变的执行请求无效或不兼容。
    #[error("runner execution specification rejected")]
    InvalidExecution,
    /// 候选被拒绝；在回收任何 scratch 之前已先保留下来供评审。
    #[error("runner candidate rejected; unlanded patch retained at {retained}")]
    CandidateRejected {
        /// 指向持久保留候选的路径；本错误不包含任何内容。
        retained: PathBuf,
    },
    /// 工作量或时间上限已耗尽。
    #[error("runner budget exhausted")]
    BudgetExceeded,
    /// 超出有界上限：补丁大小、候选文件树规模、记录大小或清理扫描的条目数。
    #[error("runner output exceeded limit")]
    OutputLimit,
    /// 启动后 CLI 异常退出，或输出了无效的事件流。
    #[error("runner protocol failed")]
    Protocol,
    /// 本适配器实例不认识该句柄。
    #[error("unknown runner handle")]
    UnknownHandle,
    /// 幂等／CAS 校验不符：同一 attempt/fence 输入不同、保留内容不一致或落地基线不符。
    #[error("runner idempotency key reused with different input")]
    IdempotencyConflict,
    /// 进程引用的 pid 或启动身份为 0（缺失）。
    #[error("process runner reference requires non-zero pid and start identity")]
    InvalidRunnerRef,
    /// 适配器状态不可用：状态互斥锁中毒，或快照根锁已被他人持有等。
    #[error("runner state is unavailable")]
    StateUnavailable,
    /// 文件系统操作失败（临时存储、快照记录、保留补丁等）。
    #[error("runner temporary storage failed during {operation}: {source}")]
    Io {
        /// 稳定的操作标签。
        operation: &'static str,
        /// 底层错误。
        #[source]
        source: std::io::Error,
    },
}

impl RunnerError {
    fn io(operation: &'static str, source: std::io::Error) -> Self {
        Self::Io { operation, source }
    }
}

/// 同步、对象安全的生命周期端口，供本地 runner 与将来的 VM runner 使用。
pub trait RunnerAdapter: Send + Sync {
    /// 报告适配器是否可用。
    fn probe(&self) -> Result<AdapterAvailability, RunnerError>;

    /// 报告当前的二进制／协议诊断信息（适配器有对应二进制时才有内容）。
    fn probe_report(&self) -> Result<AdapterProbe, RunnerError> {
        let availability = self.probe()?;
        let missing_prerequisites = match &availability {
            AdapterAvailability::Unavailable { reason } => vec![reason.clone()],
            AdapterAvailability::Ready => Vec::new(),
        };
        Ok(AdapterProbe {
            availability,
            binary_version: None,
            protocol_capabilities: Vec::new(),
            missing_prerequisites,
        })
    }

    /// 启动一次不可变的 attempt，或幂等地恢复它。
    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError>;

    /// 观测状态，并把候选产物从临时存储中复制出来。
    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError>;

    /// 幂等地撤销并隔离一次运行。
    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError>;

    /// 返回单调递增的进度计数，不延长租约。
    fn progress(&self, _handle: &RunHandle) -> Result<RunnerProgress, RunnerError> {
        Ok(RunnerProgress::default())
    }
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

/// 确定性的 M0 适配器，只写入私有临时目录。
#[derive(Default)]
pub struct StubRunner {
    state: Mutex<StubState>,
}

impl StubRunner {
    /// 创建一个空的 Stub 适配器。
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
