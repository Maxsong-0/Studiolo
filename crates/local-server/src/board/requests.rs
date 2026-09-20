use serde::Deserialize;
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId, VaultPath};
use studiolo_task_engine::{BoardReceipt, BoardTaskSpec, Handoff};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub agent_type: String,
    pub instance_id: String,
    pub project: VaultPath,
    pub operation: String,
    pub request: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    pub request_id: String,
    pub spec: BoardTaskSpec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Task {
    pub task_id: TaskId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Claim {
    pub task_id: TaskId,
    pub lease_until_ms: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub fence: Fence,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Heartbeat {
    pub identity: Identity,
    pub lease_until_ms: i64,
    pub tool_calls: u64,
    pub edits: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffRequest {
    pub identity: Identity,
    pub handoff: Handoff,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Return {
    pub identity: Identity,
    pub receipt: BoardReceipt,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Number {
    pub identity: Identity,
    pub request_id: String,
    pub scope: VaultPath,
    pub prefix: String,
}
