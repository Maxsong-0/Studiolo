//! 只读工作台与显式应用会话入口；不装配内容写引擎。

mod assets;
mod auth_http;
mod bootstrap;
mod canonical;
mod coordinator;
mod queries;
mod refresh;
mod routing;
mod security;
mod tree;
mod unix_ingress;
mod view;
mod watcher;
pub use coordinator::CanonicalWorkbench;
pub use unix_ingress::{BoundUnixIngress, IngressError, PreparedUnixIngress};

pub use routing::{
    router, router_with_access, router_with_canonical_sessions, router_with_canonical_writer,
    router_with_factors, router_with_sessions,
};
pub use studiolo_auth::{AccessConfig, AccessError, AccessIdentity, AccessVerifier};

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use studiolo_vault_broker::{BrokerError, CanonicalReadVault, ReadOnlyVault};
use studiolo_vault_index::IndexError;
use studiolo_workbench::{TreeError, WorkbenchSnapshot};
use view::{Generation, ViewStore, load};

/// 本机入口初始化失败；调用者应修正副本或路径后重试。
#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    /// 只读 Broker 拒绝读取。
    #[error("只读副本读取失败：{0}")]
    Read(#[from] BrokerError),
    /// 不完整扫描不能成为新的完整代次。
    #[error("副本扫描不完整：{0}")]
    Incomplete(String),
    /// 非 UTF-8 文档不能静默从统计中消失。
    #[error("文档不是有效 UTF-8：{0}")]
    Encoding(String),
    /// 派生索引拒绝输入。
    #[error("索引构建失败：{0}")]
    Index(#[from] IndexError),
    /// 文件清单与笔记集合不一致，拒绝发布不完整代次。
    #[error("文件树构建失败：{0}")]
    Tree(#[from] TreeError),
    /// 本机配置无效。
    #[error("本机端口或副本名称无效")]
    Configuration,
    /// 后台运行、完整捕获或生命周期暂不可用；错误不包含主机路径。
    #[error("工作台完整视图或后台运行暂不可用")]
    Unavailable,
}

#[derive(Clone)]
enum Source {
    Replica(Arc<ReadOnlyVault>),
    Writer(Arc<CanonicalWorkbench>),
}

#[derive(Clone)]
enum CanonicalSource {
    Reader(Arc<CanonicalReadVault>),
    Writer(Arc<CanonicalWorkbench>),
}

#[derive(Clone)]
struct AppState {
    source: Source,
    assets: Arc<ReadOnlyVault>,
    current: ViewStore,
    refresh: Arc<tokio::sync::Mutex<()>>,
    queries: Arc<tokio::sync::Semaphore>,
    source_label: String,
    authority: String,
    origin: String,
    access: Option<AccessVerifier>,
    sessions: Option<auth_http::AuthHttp>,
    canonical: Option<CanonicalSource>,
    #[cfg(test)]
    refresh_probe: Arc<refresh_tests::Probe>,
}

#[derive(Serialize)]
struct SnapshotResponse {
    generation: u64,
    source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    freshness: Option<view::Freshness>,
    #[serde(flatten)]
    snapshot: WorkbenchSnapshot,
}

#[derive(Default, Deserialize)]
struct SnapshotQuery {
    #[serde(default)]
    refresh: bool,
}

async fn snapshot(
    State(state): State<AppState>,
    Query(query): Query<SnapshotQuery>,
) -> Result<Json<SnapshotResponse>, ApiError> {
    if query.refresh {
        refresh::run(&state).await?;
    }
    let view = state
        .current
        .snapshot()
        .map_err(|_| ApiError::unavailable())?;
    let current = view.generation;
    Ok(Json(SnapshotResponse {
        generation: current.number,
        source_label: state.source_label,
        freshness: view.freshness,
        snapshot: current.model.snapshot(),
    }))
}

fn current(state: &AppState) -> Result<Arc<Generation>, ApiError> {
    state
        .current
        .snapshot()
        .map(|view| view.generation)
        .map_err(|_| ApiError::unavailable())
}

struct ApiError(StatusCode, String);

impl ApiError {
    fn unavailable() -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "本机视图暂不可用，请重试。".into(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

#[cfg(test)]
#[path = "../tests/refresh/private.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "../tests/coordinator/private.rs"]
mod coordinator_tests;
