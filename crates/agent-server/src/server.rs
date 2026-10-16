//! MCP 会话 handler：每个 HTTP 会话一个实例；`initialize` 铸实例并把路牌放进 instructions，
//! 全部 tools「先审计后返回」，越白名单一律拒绝并审计。
//!
//! M4 起每次调用都复核策略平面：全局／类型／实例三级吊销与该类型的能力集；供给视图 ＝
//! 部署上限 `--allow` ∩ 能力集白名单。`serve` 未给 `--policy` 时退回 M3 兼容模式（只有部署上限）。

mod args;
mod auditing;
mod board;
mod catalog;
mod content;
mod freshness;
mod grant;
mod handler;
mod policy;
mod proposals;
mod session;
mod signpost;

use std::sync::{Arc, OnceLock};

use crate::state::AppState;
use studiolo_supply::Instance;

pub use args::{
    ListEntriesArgs, ListProposalsArgs, PolicyReadArgs, ProposeArgs, ReadNoteArgs, SearchArgs,
};
pub use catalog::TOOLS;
pub(crate) use session::POLICY_DENIED_PREFIX;

/// 一个 MCP 会话。
pub struct AgentSession {
    state: Arc<AppState>,
    session_ref: String,
    instance: OnceLock<Instance>,
    http: bool,
    mcp: Option<Arc<crate::mcp_transport::Binding>>,
}
