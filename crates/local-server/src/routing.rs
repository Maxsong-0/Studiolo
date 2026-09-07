//! 显式组合入口；监听来源与领域能力由可信启动层分别选择。

mod canonical_writer;
pub use canonical_writer::router_with_canonical_writer;

use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use studiolo_auth::{FactorRuntime, SessionManager};
use studiolo_vault_broker::{CanonicalReadVault, ReadOnlyVault};

use crate::{
    AccessVerifier, AppState, CanonicalSource, LocalError, Source, ViewStore, assets, auth_http,
    load, queries, security, snapshot, tree,
};

/// 创建仅适用于 `127.0.0.1:port` 的只读路由。
///
/// 根目录预先存在；不初始化Vault或缓存，初扫须完整。只能绑定IPv4 loopback。
pub fn router(
    vault: ReadOnlyVault,
    assets: ReadOnlyVault,
    source_label: String,
    port: u16,
) -> Result<Router, LocalError> {
    configured_router(
        vault,
        assets,
        source_label,
        Endpoint::Loopback(port),
        None,
        None,
        None,
    )
}

/// 创建显式Access只读入口，包括静态文件的每个成功请求均需可信签名身份。
///
/// 只绑定IPv4 loopback；Tunnel改写Host为 `127.0.0.1:port`，Origin仅配置的HTTPS值。
pub fn router_with_access(
    vault: ReadOnlyVault,
    assets: ReadOnlyVault,
    source_label: String,
    port: u16,
    access: AccessVerifier,
) -> Result<Router, LocalError> {
    configured_router(
        vault,
        assets,
        source_label,
        Endpoint::Loopback(port),
        Some(access),
        None,
        None,
    )
}

/// 显式装配应用会话：静态引导仍每请求验Access，所有内容API另验应用Cookie配对。
///
/// 注入由可信启动层安全打开的既有Manager；本方法不建库、迁移、读种子或登记因子。
/// 新签发只在已确认登记后受理，并做一次有界回收；无后台任务或自动重试。
/// 不开放TOTP/恢复码HTTP或内容写入，不代替真实部署与浏览器验收。
/// 与原入口相同，只能绑定IPv4 loopback和固定Tunnel Host。
pub fn router_with_sessions(
    vault: ReadOnlyVault,
    assets: ReadOnlyVault,
    source_label: String,
    port: u16,
    access: AccessVerifier,
    sessions: Arc<SessionManager>,
) -> Result<Router, LocalError> {
    configured_router(
        vault,
        assets,
        source_label,
        Endpoint::Loopback(port),
        Some(access),
        Some(auth_http::AuthHttp::new(sessions)),
        None,
    )
}

/// 已有 canonical 的显式人侧读取入口；仅新增单页权威 revision，不开放写入。
///
/// `vault` 的视图与 canonical 句柄须绑定同一目录对象；普通查询仍标自己的 generation。
/// 可信启动层负责既有身份、私有存储和公开目录隔离。本入口不会初始化 Vault。
pub fn router_with_canonical_sessions(
    vault: (ReadOnlyVault, Arc<CanonicalReadVault>),
    assets: ReadOnlyVault,
    source_label: String,
    port: u16,
    access: AccessVerifier,
    sessions: Arc<SessionManager>,
) -> Result<Router, LocalError> {
    let (view, canonical) = vault;
    if view.root_identity()? != canonical.root_identity()? {
        return Err(LocalError::Configuration);
    }
    configured_router(
        view,
        assets,
        source_label,
        Endpoint::Loopback(port),
        Some(access),
        Some(auth_http::AuthHttp::new(sessions)),
        Some(canonical),
    )
}

/// 显式因子模式；只能通过本库Unix ingress提供真实连接证明。
///
/// Host固定为已验证公共Origin的authority；此路由即使误挂TCP，因子动作也拒绝无证明请求。
/// Manager和FactorRuntime须由同一个可信启动装配提供；不创建存储或读取秘密。
pub fn router_with_factors(
    vault: (ReadOnlyVault, Option<Arc<CanonicalReadVault>>),
    assets: ReadOnlyVault,
    source_label: String,
    access: AccessVerifier,
    sessions: Arc<SessionManager>,
    factors: Arc<FactorRuntime>,
) -> Result<Router, LocalError> {
    let (view, canonical) = vault;
    if let Some(canonical) = &canonical
        && view.root_identity()? != canonical.root_identity()?
    {
        return Err(LocalError::Configuration);
    }
    configured_router(
        view,
        assets,
        source_label,
        Endpoint::Unix,
        Some(access),
        Some(auth_http::AuthHttp::with_factors(sessions, factors)),
        canonical,
    )
}

enum Endpoint {
    Loopback(u16),
    Unix,
}

fn configured_router(
    vault: ReadOnlyVault,
    assets: ReadOnlyVault,
    source_label: String,
    endpoint: Endpoint,
    access: Option<AccessVerifier>,
    sessions: Option<auth_http::AuthHttp>,
    canonical: Option<Arc<CanonicalReadVault>>,
) -> Result<Router, LocalError> {
    validate(&source_label, &endpoint, access.as_ref())?;
    let current = ViewStore::replica(load(&vault, 1)?);
    assemble(
        PreparedSource {
            source: Source::Replica(Arc::new(vault)),
            current,
            canonical: canonical.map(CanonicalSource::Reader),
        },
        assets,
        source_label,
        endpoint,
        access,
        sessions,
    )
}

struct PreparedSource {
    source: Source,
    current: ViewStore,
    canonical: Option<CanonicalSource>,
}

fn validate(
    source_label: &str,
    endpoint: &Endpoint,
    access: Option<&AccessVerifier>,
) -> Result<String, LocalError> {
    if source_label.trim().is_empty()
        || source_label.chars().count() > 80
        || source_label.chars().any(char::is_control)
    {
        return Err(LocalError::Configuration);
    }
    match endpoint {
        Endpoint::Loopback(0) => return Err(LocalError::Configuration),
        Endpoint::Loopback(port) => Ok(format!("127.0.0.1:{port}")),
        Endpoint::Unix => access
            .and_then(|value| value.origin().strip_prefix("https://"))
            .map(str::to_owned)
            .ok_or(LocalError::Configuration),
    }
}

fn assemble(
    prepared: PreparedSource,
    assets: ReadOnlyVault,
    source_label: String,
    endpoint: Endpoint,
    access: Option<AccessVerifier>,
    sessions: Option<auth_http::AuthHttp>,
) -> Result<Router, LocalError> {
    let authority = validate(&source_label, &endpoint, access.as_ref())?;
    let state = AppState {
        source: prepared.source,
        assets: Arc::new(assets),
        current: prepared.current,
        refresh: Arc::new(tokio::sync::Mutex::new(())),
        queries: Arc::new(tokio::sync::Semaphore::new(2)),
        source_label,
        origin: access.as_ref().map_or_else(
            || format!("http://{authority}"),
            |value| value.origin().to_owned(),
        ),
        authority,
        access,
        sessions,
        canonical: prepared.canonical,
        #[cfg(test)]
        refresh_probe: Arc::new(crate::refresh_tests::Probe::default()),
    };
    let mut routes = Router::new()
        .route(crate::bootstrap::PATH, get(crate::bootstrap::mode))
        .route("/api/workbench", get(snapshot))
        .route("/api/document", get(queries::document))
        .route("/api/search", get(queries::search))
        .route("/api/tree", get(tree::page));
    if let Some(sessions) = &state.sessions {
        routes = routes.merge(auth_http::routes(sessions.has_factors()));
    }
    if state.canonical.is_some() {
        routes = routes.route(crate::canonical::PATH, get(crate::canonical::file));
    }
    Ok(routes
        .fallback(assets::asset)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state))
}
