//! 只保留当前装配的原实例；没有 canonical 写者就没有封存能力，不自动打开或初始化 owner。
use super::StartupError;
use std::sync::Arc;
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::SessionManager;
use studiolo_local_server::{
    CanonicalWorkbench, PolicyPlane, board::BoardPlane, checkpoint::CheckpointInputs,
    sync_plane::SyncPlane,
};

pub(super) fn sources(
    workbench: Option<Arc<CanonicalWorkbench>>,
    sessions: Option<Arc<SessionManager>>,
    audit: Option<Arc<SqliteAuditLedger>>,
    board: Option<Arc<BoardPlane>>,
    bridge: Option<Arc<SyncPlane>>,
    policy: Option<Arc<PolicyPlane>>,
) -> Result<Option<Arc<CheckpointInputs>>, StartupError> {
    let Some(workbench) = workbench else {
        return Ok(None);
    };
    Ok(Some(Arc::new(CheckpointInputs {
        workbench,
        sessions: sessions.ok_or(StartupError::Configuration)?,
        audit: audit.ok_or(StartupError::Audit)?,
        board,
        bridge,
        policy,
    })))
}

// 仅注入给既有可信路由；本步骤不增设公开端点、调度器或后台冻结。
pub(super) fn attach(app: axum::Router, input: Option<Arc<CheckpointInputs>>) -> axum::Router {
    match input {
        Some(input) => app.layer(axum::Extension(input)),
        None => app,
    }
}
