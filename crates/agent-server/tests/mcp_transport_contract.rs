//! 整个 MCP 传输的真实入口契约，不能只依靠 tools 内的原实例检查。
use serde_json::{Value, json};
use std::{
    fs, net::SocketAddr, os::unix::fs::PermissionsExt, path::Path, sync::Arc, time::Duration,
};
use studiolo_agent_server::{options::ServeOptions, serve, state::AppState};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::AgentStore;
use tempfile::TempDir;

include!("mcp_contract_support/fixture.rs");

fn transport(
    f: &Fixture,
    method: reqwest::Method,
    token: &str,
    session: &str,
) -> reqwest::RequestBuilder {
    f.client
        .request(method, &f.base)
        .bearer_auth(token)
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", session)
        .header("MCP-Protocol-Version", "2025-11-25")
        .header("Content-Type", "application/json")
}

include!("mcp_transport_cases/identity.rs");
include!("mcp_transport_cases/lifecycle.rs");
include!("mcp_transport_cases/input.rs");
include!("mcp_transport_cases/session.rs");
include!("mcp_transport_cases/identity_store.rs");
