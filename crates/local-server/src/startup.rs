//! 进程组合根：检查全部成功后才允许监听；HTTP不持有配置或初始化能力。

mod canonical_input;
mod error;
mod factor_input;
mod health_input;
mod repository_input;
mod serving;

pub(crate) use error::StartupError;

use std::{future::Future, io::Write, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    AccessConfig, AccessError, AccessVerifier, OsSessionRandom, SessionManager, SystemSessionClock,
};
use studiolo_host_config::{Config, ServingRoots};
use studiolo_local_server::ProjectRepository;
use studiolo_local_server::{CanonicalWorkbench, PreparedUnixIngress};
use studiolo_vault_broker::{CanonicalReadVault, ReadOnlyVault};

use crate::options::Options;

pub(crate) struct PreparedServer {
    app: axum::Router,
    port: u16,
    mode: &'static str,
    roots: Option<ServingRoots>,
    canonical: Option<Arc<CanonicalReadVault>>,
    ingress: Option<PreparedUnixIngress>,
    workbench: Option<Arc<CanonicalWorkbench>>,
    repository: Option<Arc<ProjectRepository>>,
}

impl PreparedServer {
    /// 生产唯一入口始终使用真实的固定Cloudflare取钥实现。
    pub(crate) async fn prepare(options: Options) -> Result<Self, StartupError> {
        prepare_with(options, AccessVerifier::connect).await
    }
}

// 生产固定真实取钥和固定秘密槽；测试Seam不公开回调、URL覆盖或身份工厂。
async fn prepare_with<F, Fut>(options: Options, connect: F) -> Result<PreparedServer, StartupError>
where
    F: FnOnce(AccessConfig) -> Fut,
    Fut: Future<Output = Result<AccessVerifier, AccessError>>,
{
    prepare_composed(options, connect, factor_input::environment).await
}

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
    let residency = studiolo_host_config::process_safety::harden()?;
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
    {
        return Err(StartupError::Configuration);
    }
    let mut repository = repository_input::open(&options)?;
    let mut content_audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>> = None;
    let (sessions, roots, access_config, factors, writer_roots) =
        if let Some(session) = &options.session {
            let config = Config::load(&session.config).map_err(|_| StartupError::Configuration)?;
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
            (
                Some(manager),
                Some(roots),
                Some(access),
                factors,
                writer_roots,
            )
        } else {
            (None, None, options.access, None, None)
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
    let (app, mode) = if let Some(config) = access_config {
        let access = connect(config).await.map_err(|_| StartupError::Access)?;
        if let Some(writer) = writer {
            workbench = Some(writer.prepare(content_audit).await?);
        }
        if let Some(sessions) = sessions {
            if let Some(workbench) = &workbench {
                (
                    studiolo_local_server::router_with_canonical_writer(
                        Arc::clone(workbench),
                        assets,
                        options.label,
                        options.port,
                        access,
                        sessions,
                        factors,
                    ),
                    "Access及应用会话；canonical完整监视只读",
                )
            } else if let Some(factors) = factors {
                (
                    studiolo_local_server::router_with_factors(
                        (vault, canonical.as_ref().map(Arc::clone)),
                        assets,
                        options.label,
                        access,
                        sessions,
                        factors,
                    ),
                    "Access、应用会话及二次认证；可信Unix入口",
                )
            } else if let Some(canonical) = &canonical {
                (
                    studiolo_local_server::router_with_canonical_sessions(
                        (vault, Arc::clone(canonical)),
                        assets,
                        options.label,
                        options.port,
                        access,
                        sessions,
                    ),
                    "Access及已登记应用会话；canonical只读",
                )
            } else {
                (
                    studiolo_local_server::router_with_sessions(
                        vault,
                        assets,
                        options.label,
                        options.port,
                        access,
                        sessions,
                    ),
                    "Access及已登记应用会话；只读",
                )
            }
        } else {
            (
                studiolo_local_server::router_with_access(
                    vault,
                    assets,
                    options.label,
                    options.port,
                    access,
                ),
                "公共示例入口已启用Access鉴权；只读",
            )
        }
    } else {
        (
            studiolo_local_server::router(vault, assets, options.label, options.port),
            "本机只读",
        )
    };
    let app = match app {
        Ok(app) => app,
        Err(_) => {
            canonical_input::shutdown(workbench.as_ref()).await?;
            return Err(StartupError::Router);
        }
    };
    let app = if let Some(repository) = &repository {
        studiolo_local_server::with_project_repository(app, Arc::clone(repository))
    } else {
        app
    };
    // 健康页的路由已在 router 装配期挂好（且在鉴权那一层之内）；这里只把证据来源
    // 装上去。不装则健康页一律回 404，因此这一行是"健康页可用"的唯一开关。
    let app = studiolo_local_server::health::with_health_evidence(app, health_input::source());
    let prepared = PreparedServer {
        app,
        port: options.port,
        mode,
        roots,
        canonical,
        ingress,
        workbench,
        repository,
    };
    if let Err(error) = prepared.revalidate() {
        canonical_input::shutdown(prepared.workbench.as_ref()).await?;
        return Err(error);
    }
    Ok(prepared)
}

#[cfg(test)]
#[path = "../tests/composition/mod.rs"]
mod tests;
