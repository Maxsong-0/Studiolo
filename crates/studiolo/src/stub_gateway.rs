//! 合成的 M6 基线：候选字节来自经授权的 `Demo` 工具响应。
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use studiolo_core::{ToolId, VaultPath};
use studiolo_gateway::{CallId, Gateway, GatewayError, JobCapability, ToolCall, ToolResult};
use studiolo_runner_adapter::{
    AdapterAvailability, RunHandle, RunInput, RunSpec, RunnerAdapter, RunnerArtifact, RunnerError,
    RunnerObservation, RunnerProgress, StartedRun, StubRunner,
};

#[derive(Default)]
struct State {
    spec: Option<RunSpec>,
    started: Option<StartedRun>,
    results: Option<Vec<ToolResult>>,
    cancelled: bool,
}

/// 只跑一次尝试的 `Demo` 版 `runner`，与其他适配器共用同一个 `Gateway` trait。
/// 只能用受信任的 `Gateway` 与经组合根校验过的能力来构造。
/// 每次观察或重放时，`Gateway` 都会重新检查权限、过期、取消与 `fence`。
pub struct GatewayStubRunner {
    gateway: Arc<dyn Gateway>,
    capability: JobCapability,
    inner: StubRunner,
    state: Mutex<State>,
}
impl GatewayStubRunner {
    /// 把这个合成适配器绑定到一份已校验过的 `attempt` 能力上。
    #[must_use]
    pub fn new(gateway: Arc<dyn Gateway>, capability: JobCapability) -> Self {
        Self {
            gateway,
            capability,
            inner: StubRunner::new(),
            state: Mutex::new(State::default()),
        }
    }
    fn tools(&self) -> Result<Vec<ToolResult>, GatewayError> {
        ["web.fetch", "db.query"]
            .into_iter()
            .map(|name| {
                let call = ToolCall::new(
                    CallId::parse(format!(
                        "demo-v1:{}:{}:{}:{name}",
                        self.capability.task_id(),
                        self.capability.attempt_id(),
                        self.capability.fence().get()
                    ))?,
                    ToolId::from_static(name),
                    demo_arguments(name),
                );
                let result = self.gateway.invoke(&self.capability, call.clone())?;
                if result.call_id() != call.call_id() || result.tool() != call.tool() {
                    return Err(GatewayError::CallConflict);
                }
                Ok(result)
            })
            .collect()
    }
    fn known(state: &State, handle: &RunHandle) -> Result<(), RunnerError> {
        if state
            .started
            .as_ref()
            .is_some_and(|run| &run.handle == handle)
        {
            Ok(())
        } else {
            Err(RunnerError::UnknownHandle)
        }
    }
}

pub(crate) fn demo_arguments(tool: &str) -> Value {
    match tool {
        "web.fetch" => json!({"target_id":"demo-web","path":"/demo.json","query":{}}),
        "db.query" => json!({"source_id":"demo-db","view":"demo_items","limit":32}),
        _ => Value::Null,
    }
}

impl RunnerAdapter for GatewayStubRunner {
    fn probe(&self) -> Result<AdapterAvailability, RunnerError> {
        self.inner.probe()
    }
    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError> {
        if spec.task_id != self.capability.task_id()
            || spec.attempt_id != self.capability.attempt_id()
            || spec.fence != self.capability.fence()
            || !spec.budget.valid()
            || spec.budget.max_tools < 2
        {
            return Err(RunnerError::InvalidExecution);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        if state.spec.as_ref().is_some_and(|prior| prior != spec) {
            return Err(RunnerError::IdempotencyConflict);
        }
        if state.cancelled {
            return Err(RunnerError::Unavailable);
        }
        state.spec = Some(spec.clone());
        let results = self.tools().map_err(|_| RunnerError::Unavailable)?;
        if state
            .results
            .as_ref()
            .is_some_and(|prior| prior != &results)
        {
            return Err(RunnerError::IdempotencyConflict);
        }
        let mut effective = spec.clone();
        for result in &results {
            let content = serde_json::to_vec(result).map_err(|_| RunnerError::Protocol)?;
            if content.len() > 128 * 1024 {
                return Err(RunnerError::OutputLimit);
            }
            effective.inputs.push(RunInput {
                path: VaultPath::parse(format!("demo/{}.json", result.tool().as_str()))
                    .map_err(|_| RunnerError::Protocol)?,
                revision: result.call_id().as_str().into(),
                content,
            });
        }
        let started = self.inner.start(&effective)?;
        state.results = Some(results);
        state.started = Some(started.clone());
        Ok(started)
    }
    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        Self::known(&state, handle)?;
        if state.cancelled {
            return Ok(RunnerObservation::Cancelled);
        }
        let results = self.tools().map_err(|_| RunnerError::Unavailable)?;
        if state.results.as_ref() != Some(&results) {
            return Err(RunnerError::IdempotencyConflict);
        }
        let mut observation = self.inner.observe(handle)?;
        if let RunnerObservation::Completed(report) = &mut observation {
            report.artifacts.push(RunnerArtifact {
                name: VaultPath::parse("gateway-demo.json").map_err(|_| RunnerError::Protocol)?,
                content: serde_json::to_vec(
                    &json!({"format":"studiolo.gateway-demo.v1","mode":"demo",
                    "task_id":report.task_id,"attempt_id":report.attempt_id,"fence":report.fence,
                    "results":results}),
                )
                .map_err(|_| RunnerError::Protocol)?,
            });
        }
        Ok(observation)
    }
    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        Self::known(&state, handle)?;
        self.inner.cancel(handle)?;
        state.cancelled = true;
        Ok(())
    }
    fn progress(&self, handle: &RunHandle) -> Result<RunnerProgress, RunnerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RunnerError::StateUnavailable)?;
        Self::known(&state, handle)?;
        Ok(RunnerProgress {
            tool_calls: 2,
            ..RunnerProgress::default()
        })
    }
}
