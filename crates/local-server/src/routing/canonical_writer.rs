//! 写者模式只公开现有读取路由及新鲜度，监听来源仍由既有会话/因子规则决定。

use std::sync::Arc;

use axum::Router;
use studiolo_auth::{FactorRuntime, SessionManager};
use studiolo_vault_broker::ReadOnlyVault;

use super::{Endpoint, PreparedSource, assemble};
use crate::{AccessVerifier, CanonicalSource, CanonicalWorkbench, LocalError, Source, auth_http};

/// 已完成首次捕获的显式 canonical 工作台；不提供保存或任意 Broker 写能力。
///
/// 因子存在时仅适配可信 Unix ingress，否则固定 loopback；bootstrap 仍只有认证字段。
pub fn router_with_canonical_writer(
    workbench: Arc<CanonicalWorkbench>,
    assets: ReadOnlyVault,
    source_label: String,
    port: u16,
    access: AccessVerifier,
    sessions: Arc<SessionManager>,
    factors: Option<Arc<FactorRuntime>>,
) -> Result<Router, LocalError> {
    workbench.revalidate()?;
    if assets.root_identity()? != workbench.core.roots.assets_identity() {
        return Err(LocalError::Configuration);
    }
    let (endpoint, sessions) = match factors {
        Some(factors) => (
            Endpoint::Unix,
            auth_http::AuthHttp::with_factors(sessions, factors),
        ),
        None => (Endpoint::Loopback(port), auth_http::AuthHttp::new(sessions)),
    };
    assemble(
        PreparedSource {
            current: workbench.core.view.clone(),
            source: Source::Writer(Arc::clone(&workbench)),
            canonical: Some(CanonicalSource::Writer(workbench)),
        },
        assets,
        source_label,
        endpoint,
        Some(access),
        Some(sessions),
    )
}
