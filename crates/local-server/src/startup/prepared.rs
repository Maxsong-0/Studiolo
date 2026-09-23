//! 已装配、尚未监听的服务器与生产唯一入口；组合顺序在 `startup.rs`，收尾在 `serving.rs`。

use super::{StartupError, factor_input, prepare_composed};
use std::{future::Future, sync::Arc};
use studiolo_auth::{AccessConfig, AccessError, AccessVerifier};
use studiolo_host_config::ServingRoots;
use studiolo_local_server::ProjectRepository;
use studiolo_local_server::{CanonicalWorkbench, PreparedUnixIngress};
use studiolo_vault_broker::CanonicalReadVault;

use crate::options::Options;

pub(crate) struct PreparedServer {
    pub(super) app: axum::Router,
    pub(super) port: u16,
    pub(super) mode: &'static str,
    pub(super) roots: Option<ServingRoots>,
    pub(super) canonical: Option<Arc<CanonicalReadVault>>,
    pub(super) ingress: Option<PreparedUnixIngress>,
    pub(super) workbench: Option<Arc<CanonicalWorkbench>>,
    pub(super) repository: Option<Arc<ProjectRepository>>,
    pub(super) runner_plane: Option<Arc<studiolo_local_server::board::RunnerPlane>>,
    pub(super) board_ingress: Option<PreparedUnixIngress>,
    pub(super) board_plane: Option<Arc<studiolo_local_server::board::BoardPlane>>,
    pub(super) sync_plane: Option<Arc<studiolo_local_server::sync_plane::SyncPlane>>,
}

impl PreparedServer {
    /// 生产唯一入口始终使用真实的固定Cloudflare取钥实现。
    pub(crate) async fn prepare(options: Options) -> Result<Self, StartupError> {
        prepare_with(options, AccessVerifier::connect).await
    }
}

// 生产固定真实取钥和固定秘密槽；测试Seam不公开回调、URL覆盖或身份工厂。
pub(crate) async fn prepare_with<F, Fut>(
    options: Options,
    connect: F,
) -> Result<PreparedServer, StartupError>
where
    F: FnOnce(AccessConfig) -> Fut,
    Fut: Future<Output = Result<AccessVerifier, AccessError>>,
{
    prepare_composed(options, connect, factor_input::environment).await
}
