//! 可选平面装配：策略、黑板与执行平面只在显式参数齐备时装上，缺项一律拒绝启动。
//!
//! 这里只做纯装配，不关停写者；任一步失败由组合根先关停已启动的写者再返回。

use super::StartupError;
use crate::options::Options;
use std::{path::PathBuf, sync::Arc};
use studiolo_audit_ledger::AuditLedger;
use studiolo_local_server::{
    CanonicalWorkbench, PreparedUnixIngress,
    board::{BoardPlane, RunnerConfig, RunnerPlane},
};

/// 策略平面与健康页同一形态：会话模式下显式给了 --policy-data 才打开主人发布者句柄
/// 并装上；事件进人侧权威账本。缺这一步策略路由一律回 404。
pub(super) fn policy(
    app: axum::Router,
    options: &Options,
    audit: Option<&Arc<dyn AuditLedger>>,
    protected: &[PathBuf],
) -> Result<
    (
        axum::Router,
        Option<Arc<studiolo_local_server::PolicyPlane>>,
    ),
    StartupError,
> {
    match (&options.policy_data, audit) {
        (Some(directory), Some(ledger)) => {
            let store = studiolo_policy::PolicyStore::open(directory)
                .map_err(|_| StartupError::Configuration)?;
            let mut plane = studiolo_local_server::PolicyPlane::new(store, Arc::clone(ledger));
            if let Some(directory) = &options.agent_data {
                let isolated = identity_isolated_roots(options, protected);
                let identities = studiolo_supply::AgentIdentityView::open_existing_isolated(
                    directory, &isolated,
                )
                .map_err(|_| StartupError::Configuration)?;
                let audit = identities
                    .read_audit()
                    .map_err(|_| StartupError::Configuration)?;
                plane = plane.with_read_audit(audit).with_identities(identities);
            }
            let plane = Arc::new(plane);
            Ok((
                studiolo_local_server::with_policy_plane(app, plane.clone()),
                Some(plane),
            ))
        }
        (Some(_), None) => Err(StartupError::Configuration),
        (None, _) => Ok((app, None)),
    }
}

/// 黑板平面及其私有 Unix 入口；黑板参数、写者、审计、策略四项缺一即拒绝。
pub(super) fn board(
    options: &Options,
    workbench: Option<&Arc<CanonicalWorkbench>>,
    audit: Option<&Arc<dyn AuditLedger>>,
    protected: &[PathBuf],
    runner: Option<&RunnerConfig>,
) -> Result<(Option<PreparedUnixIngress>, Option<Arc<BoardPlane>>), StartupError> {
    match (&options.board, workbench, audit, &options.policy_data) {
        (Some(board), Some(writer), Some(audit), Some(policy)) => {
            let mut isolated = vec![
                options.vault.as_path(),
                options.assets.as_path(),
                board.agent_data.as_path(),
                policy.as_path(),
                options
                    .writer_data
                    .as_ref()
                    .ok_or(StartupError::Configuration)?
                    .as_path(),
            ];
            isolated.extend(protected.iter().map(|p| p.as_path()));
            let plane = BoardPlane::open_with_runner(
                writer.clone(),
                &board.data,
                &board.agent_data,
                policy,
                &isolated,
                audit.clone(),
                runner,
            )
            .map_err(|_| StartupError::Configuration)?;
            isolated.push(board.data.as_path());
            let ingress =
                PreparedUnixIngress::prepare_over_leftover(&board.socket, &isolated, "黑板入口")
                    .map_err(|_| StartupError::Ingress)?;
            Ok((Some(ingress), Some(plane)))
        }
        (None, _, _, _) => Ok((None, None)),
        _ => Err(StartupError::Configuration),
    }
}

/// 执行平面只在黑板平面与执行配置同时就绪时打开；只给执行配置而无黑板即拒绝。
pub(super) fn runner(
    board: Option<&Arc<BoardPlane>>,
    config: Option<RunnerConfig>,
) -> Result<Option<Arc<RunnerPlane>>, StartupError> {
    match (board, config) {
        (Some(board), Some(config)) => RunnerPlane::open(board.clone(), config)
            .map(Some)
            .map_err(|_| StartupError::Configuration),
        (_, None) => Ok(None),
        _ => Err(StartupError::Configuration),
    }
}

// 组合根只列出已批准的根；规范化与隔离核验由供给存储适配器执行。
fn identity_isolated_roots<'a>(
    options: &'a Options,
    protected: &'a [PathBuf],
) -> Vec<&'a std::path::Path> {
    [
        Some(options.vault.as_path()),
        Some(options.assets.as_path()),
        options.policy_data.as_deref(),
        options.writer_data.as_deref(),
        options.board.as_ref().map(|b| b.data.as_path()),
    ]
    .into_iter()
    .flatten()
    .chain(protected.iter().map(PathBuf::as_path))
    .collect()
}
#[cfg(test)]
#[path = "../../tests/composition/identity_input.rs"]
mod identity_tests;
