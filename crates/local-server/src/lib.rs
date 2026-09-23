//! 工作台、显式人侧正文会话与本机事务维护；canonical 写入始终交给唯一 Broker。

mod assets;
mod auth_http;
pub mod backup;
pub mod board;
mod bootstrap;
mod canonical;
mod coordinator;
mod editing;
mod file_operation_audit;
mod file_operations;
pub mod health;
mod history;
pub mod maintenance;
pub mod policy_http;
mod project_cargo;
mod project_operations;
mod project_repository;
mod project_rust;
mod queries;
mod refresh;
mod routing;
mod security;
#[cfg(test)]
mod sync_transport_config;
pub mod sync_transport_owner;
mod tree;
mod unix_ingress;
mod view;
mod watcher;
pub use coordinator::CanonicalWorkbench;
pub use policy_http::{PolicyPlane, with_policy_plane};
pub use project_repository::{ProjectRepository, with_project_repository};
pub use unix_ingress::{BoundUnixIngress, IngressError, PreparedUnixIngress};

pub use routing::{
    router, router_with_access, router_with_canonical_sessions, router_with_canonical_writer,
    router_with_factors, router_with_sessions,
};
pub use studiolo_auth::{AccessConfig, AccessError, AccessIdentity, AccessVerifier};

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use studiolo_vault_broker::{BrokerError, CanonicalReadVault, ReadOnlyVault};
use studiolo_vault_index::IndexError;
use studiolo_workbench::TreeError;
use view::{Generation, ViewStore, load};

/// 本机入口初始化失败；调用者应修正副本或路径后重试。
#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    /// 只读 Broker 拒绝读取。
    #[error("只读副本读取失败：{0}")]
    Read(#[from] BrokerError),
    /// 不完整扫描不能成为新的完整代次；诊断只保留有界摘录并注明总数。
    #[error("副本扫描不完整（共 {total} 条诊断）：{listed}")]
    Incomplete {
        /// 本次扫描的诊断总条数。
        total: usize,
        /// 有界摘录：最多列前若干条、每条截断，其余只以条数提示。
        listed: String,
    },
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
    refresh: Arc<refresh::RefreshCoalescer>,
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

#[derive(Default, Deserialize)]
struct SnapshotQuery {
    #[serde(default)]
    refresh: bool,
}

async fn snapshot(
    State(state): State<AppState>,
    query: Result<Query<SnapshotQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return invalid_query("invalid_workbench_query");
    };
    refreshed_overview(state, query.refresh)
        .await
        .into_response()
}

/// 显式要求时先合流刷新再投影首屏；刷新失败直接答复，不拿旧代次冒充新结果。
async fn refreshed_overview(
    state: AppState,
    refresh: bool,
) -> Result<Json<queries::OverviewResponse>, ApiError> {
    if refresh {
        refresh::run(&state).await?;
    }
    queries::overview(state).await
}

/// 查询串反序列化失败的统一答复：固定中文 JSON Problem，错误码按路由区分。
///
/// axum 默认拒绝是英文纯文本 400，并把缺失或畸形的字段名写进正文；这里一律不回显。
fn invalid_query(code: &'static str) -> Response {
    editing::Problem::new(StatusCode::BAD_REQUEST, code, "查询参数不合法。").into_response()
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
