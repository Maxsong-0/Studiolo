//! 进程组合根：检查全部成功后才允许监听；HTTP不持有配置或初始化能力。

mod canonical_input;
mod error;
mod factor_input;
mod health_input;
mod plane_input;
mod prepared;
mod repository_input;
mod router_input;
mod runner_input;
mod serving;
mod sync_input;

pub(crate) use error::StartupError;
pub(crate) use prepared::PreparedServer;
#[cfg(test)]
pub(crate) use prepared::prepare_with;

use std::{future::Future, io::Write, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    AccessConfig, AccessError, AccessVerifier, OsSessionRandom, SessionManager, SystemSessionClock,
};
use studiolo_host_config::{Config, ServingRoots};
use studiolo_local_server::CanonicalWorkbench;
use studiolo_vault_broker::{CanonicalReadVault, ReadOnlyVault};

use crate::options::Options;

async fn prepare_composed<F, Fut, E>(
    options: Options,
    connect: F,
    environment: E,
) -> Result<PreparedServer, StartupError>
where
    F: FnOnce(AccessConfig) -> Fut,
    Fut: Future<Output = Result<AccessVerifier, AccessError>>,
    E: FnOnce() -> factor_input::Environment,
{
    // 秘密驻留面在这里收紧，位置是本进程组合根的第一条语句，**与 options.factor 无关**：
    // 非因子模式同样在内存里持有 SessionToken / CsrfToken 明文，崩溃转储、在线检视与
    // 换出这三条路径对它们一视同仁；把收紧埋在因子输入里，非因子模式就一项也没有。
    // 必须早于任何秘密读取、任何存储打开，也早于任何可能派生的子进程（本 crate 由
    // 架构守卫禁用 `Command::new`，因此这里就是全进程最早的可用位置）。
    // 逐项分类随即记录到标准错误：只含措施名与固定分类，永不含种子或任何环境值；
    // 只返回不记录等于把本平台的「不支持」藏起来。本平台声明支持却没能生效时整体
    // 拒绝启动，此时尚未读取任何秘密、也尚未打开任何存储，不降级成「没成功也照跑」。
    let residency = studiolo_process_safety::harden()?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}")
        .map_err(|_| StartupError::ResidencyReport)?;
    if options.port == 0
        || options.label.trim().is_empty()
        || options.label.chars().count() > 80
        || options.label.chars().any(char::is_control)
        || (options.session.is_some() && options.access.is_some())
        || (options.canonical && options.session.is_none())
        || (options.factor.is_some() && options.session.is_none())
        || (options.project_repository.is_some() && options.session.is_some())
        || (options.cargo_dependencies && options.project_repository.is_none())
        || (options.rust_call_sites.is_some() && options.project_repository.is_none())
        || (options.writer_data.is_some() && (options.canonical || options.session.is_none()))
        || options
            .writer_data
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        || options
            .policy_data
            .as_ref()
            .is_some_and(|path| !path.is_absolute() || options.session.is_none())
        || options.runner_config.as_ref().is_some_and(|path| {
            !path.is_absolute() || options.board.is_none() || options.factor.is_none()
        })
    {
        return Err(StartupError::Configuration);
    }
    let runner_config = options
        .runner_config
        .as_deref()
        .map(runner_input::load)
        .transpose()?;
    let mut repository = repository_input::open(&options)?;
    let mut content_audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>> = None;
    let mut board_protected = Vec::new();
    let (sessions, roots, access_config, factors, writer_roots, sync_scope) =
        if let Some(session) = &options.session {
            let config = Config::load(&session.config).map_err(|_| StartupError::Configuration)?;
            board_protected.extend([
                config.auth_directory().to_owned(),
                config.audit_directory().to_owned(),
                session
                    .config
                    .parent()
                    .ok_or(StartupError::Configuration)?
                    .to_owned(),
            ]);
            repository = repository_input::approved(&options, &config)?;
            let access = config
                .access(&session.origin)
                .map_err(|_| StartupError::Configuration)?;
            let roots = config
                .serving_roots(&options.vault, &options.assets)
                .map_err(|_| StartupError::Configuration)?;
            let writer_roots = options
                .writer_data
                .as_ref()
                .map(|data| {
                    config
                        .canonical_serving_roots(&options.vault, &options.assets, data)
                        .map(Arc::new)
                        .map_err(|_| StartupError::Configuration)
                })
                .transpose()?;
            let factor_input = options
                .factor
                .as_ref()
                .map(|factor| {
                    factor_input::Inputs::load(
                        factor,
                        &config,
                        &session.config,
                        &options.vault,
                        &options.assets,
                        options.writer_data.as_deref(),
                        environment,
                    )
                })
                .transpose()?;
            let audit = Arc::new(
                SqliteAuditLedger::open_existing(config.audit_directory())
                    .map_err(|_| StartupError::Audit)?,
            );
            let manager = Arc::new(SessionManager::open_existing(
                config.auth_directory(),
                config.sessions().map_err(|_| StartupError::Configuration)?,
                {
                    content_audit = Some(audit.clone());
                    audit
                },
                Arc::new(SystemSessionClock),
                Arc::new(OsSessionRandom),
            )?);
            manager.require_http_ready()?;
            let factors = factor_input
                .map(|inputs| inputs.attach(Arc::clone(&manager)))
                .transpose()?;
            // 双向 Bridge 的范围批准也来自同一份私有配置；缺字段即平面关闭。
            let sync_scope = config
                .sync_scope()
                .map_err(|_| StartupError::Configuration)?;
            (
                Some(manager),
                Some(roots),
                Some(access),
                factors,
                writer_roots,
                sync_scope,
            )
        } else {
            // 复制而非移走：后面的可选平面装配还要整体借用 options。
            (None, None, options.access.clone(), None, None, None)
        };
    let writer = writer_roots
        .map(canonical_input::Inputs::open)
        .transpose()?;
    let vault = ReadOnlyVault::open(&options.vault).map_err(|_| StartupError::ReadRoots)?;
    let assets = ReadOnlyVault::open(&options.assets).map_err(|_| StartupError::ReadRoots)?;
    let canonical = if options.canonical {
        Some(Arc::new(
            CanonicalReadVault::open_existing(&options.vault)
                .map_err(|_| StartupError::ReadRoots)?,
        ))
    } else {
        None
    };
    if let Some(roots) = &roots {
        if vault.root_identity().map_err(|_| StartupError::ReadRoots)? != roots.vault_identity()
            || assets
                .root_identity()
                .map_err(|_| StartupError::ReadRoots)?
                != roots.assets_identity()
        {
            return Err(StartupError::ReadRoots);
        }
        roots
            .revalidate()
            .map_err(|_| StartupError::Configuration)?;
    }
    let (factors, ingress) = factors.map_or((None, None), |prepared| {
        (Some(prepared.runtime), Some(prepared.ingress))
    });
    let mut workbench = None;
    let access = if let Some(config) = access_config {
        let access = connect(config).await.map_err(|_| StartupError::Access)?;
        if let Some(writer) = writer {
            workbench = Some(writer.prepare(content_audit.clone()).await?);
        }
        Some(access)
    } else {
        None
    };
    // 装配失败时 compose 先关停写者，再带着不含秘密的原因类别返回。
    let (app, mode) = router_input::compose(router_input::Inputs {
        vault,
        assets,
        label: options.label.clone(),
        port: options.port,
        access,
        sessions,
        factors,
        canonical: canonical.clone(),
        workbench: workbench.clone(),
    })
    .await?;
    let app = if let Some(repository) = &repository {
        studiolo_local_server::with_project_repository(app, Arc::clone(repository))
    } else {
        app
    };
    let policy = plane_input::policy(app, &options, content_audit.as_ref());
    let app = or_shutdown(policy, workbench.as_ref()).await?;
    let board = plane_input::board(
        &options,
        workbench.as_ref(),
        content_audit.as_ref(),
        &board_protected,
        runner_config.as_ref(),
    );
    let (board_ingress, board_plane) = or_shutdown(board, workbench.as_ref()).await?;
    let app = if let Some(plane) = &board_plane {
        studiolo_local_server::board::with_board_plane(app, plane.clone())
    } else {
        app
    };
    let runner = plane_input::runner(board_plane.as_ref(), runner_config);
    let runner_plane = or_shutdown(runner, workbench.as_ref()).await?;
    let app = if let Some(runner) = &runner_plane {
        app.layer(axum::Extension(runner.clone()))
    } else {
        app
    };
    // 双向 Bridge 平面（ADR-025 D1）：默认关闭，只有显式审批文档加范围批准才装配。
    let sync = sync_input::assemble(
        &options,
        sync_scope,
        roots.as_ref().map(ServingRoots::vault_identity),
        workbench.as_ref(),
        content_audit.as_ref(),
        &board_protected,
    );
    let sync_plane = or_shutdown(sync, workbench.as_ref()).await?;
    // 健康页的路由已在 router 装配期挂好（且在鉴权那一层之内）；这里只把证据来源
    // 装上去。不装则健康页一律回 404，因此这一行是"健康页可用"的唯一开关。
    let app = studiolo_local_server::health::with_health_evidence(
        app,
        health_input::source(sync_plane.as_ref().map(|plane| plane.health_probe())),
    );
    let prepared = PreparedServer {
        app,
        port: options.port,
        mode,
        roots,
        canonical,
        ingress,
        workbench,
        repository,
        board_ingress,
        board_plane,
        runner_plane,
        sync_plane,
    };
    if let Err(error) = prepared.revalidate() {
        canonical_input::shutdown(prepared.workbench.as_ref()).await?;
        return Err(error);
    }
    Ok(prepared)
}

/// 写者就绪之后的装配步骤：失败时先关停已启动的写者，再原样返回该步骤的错误。
async fn or_shutdown<T>(
    step: Result<T, StartupError>,
    workbench: Option<&Arc<CanonicalWorkbench>>,
) -> Result<T, StartupError> {
    if step.is_err() {
        canonical_input::shutdown(workbench).await?;
    }
    step
}

#[cfg(test)]
#[path = "../tests/composition/mod.rs"]
mod tests;
