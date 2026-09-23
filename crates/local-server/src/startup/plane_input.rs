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
) -> Result<axum::Router, StartupError> {
    match (&options.policy_data, audit) {
        (Some(directory), Some(ledger)) => studiolo_policy::PolicyStore::open(directory)
            .map_err(|_| StartupError::Configuration)
            .map(|store| {
                studiolo_local_server::with_policy_plane(
                    app,
                    Arc::new(studiolo_local_server::PolicyPlane::new(
                        store,
                        Arc::clone(ledger),
                    )),
                )
            }),
        (Some(_), None) => Err(StartupError::Configuration),
        (None, _) => Ok(app),
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
            let ingress = PreparedUnixIngress::prepare(&board.socket, &isolated)
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
