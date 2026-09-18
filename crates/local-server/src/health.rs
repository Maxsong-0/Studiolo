//! 健康页 HTTP 入口：本地只读聚合，**不新增任何无鉴权对外入口**。
//!
//! 路径固定在 `/api/v1/health`，因此与现有内容 API 落在同一条门禁上：
//! `security::guard` 对 `/api/` 前缀强制走 `auth_http::authorize`，
//! 会话与 CSRF 语义与既有入口完全一致。装配层只在**已装配会话**时才允许挂载
//! 本路由（见 [`routes`]），没有会话就根本不存在这个端点。
//!
//! 证据来源由**可信启动层**在路由装配完成后用 [`with_health_evidence`] 装上，
//! 与既有 `with_project_repository` 同一种接线形态。没装证据来源时本入口
//! 一律回 404（见 [`enabled`]），对客户端而言等同于"这个端点不存在"，
//! 绝不会出现一个"看起来正常"的空健康页。
//!
//! 本 Module 不读磁盘、不查 Sync、不连远端，也不修改任何配置。

mod evidence;
mod model;

pub use evidence::{EvidenceSnapshot, HealthEvidence};
pub use model::{HealthBody, SectionBody};

use axum::{
    Extension, Json, Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use std::sync::Arc;

/// 健康页固定路径；`/api/` 前缀让它自动落在既有鉴权门禁内。
pub const PATH: &str = "/api/v1/health";

/// 已装会话门禁的路由上，证据来源的持有者。
///
/// 只可能由 [`with_health_evidence`] 构造，因此"页面上有结论"永远等价于
/// "可信启动层确实装配过证据来源"。
#[derive(Clone)]
pub struct HealthEvidenceSource(Arc<dyn HealthEvidence>);

/// 只在既有会话门禁存在、且证据来源已装配时才可用。
///
/// 两个条件缺一不可：缺会话就等于新增无鉴权入口，缺证据来源就连"未知"都无从谈起。
#[must_use]
pub fn enabled(sessions_present: bool, evidence_present: bool) -> bool {
    sessions_present && evidence_present
}

/// 由组合根调用：装配了会话门禁时挂载只读健康页，否则原样返回。
///
/// `sessions_present` 必须如实反映本次装配是否已装配应用会话门禁。
/// 传 `false` 时本函数**拒绝挂载**，因此不可能出现无鉴权的健康入口。
/// 本函数必须在 `security::guard` 那一层之前调用，挂上来的路由才受门禁保护。
pub fn routes<S>(routes: Router<S>, sessions_present: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // 证据来源那一半由启动层在运行期补齐，缺它时 `page` 一律回 404。
    if enabled(sessions_present, true) {
        routes.route(PATH, get(page))
    } else {
        routes
    }
}

/// 由可信启动层调用：把已构造的证据来源装到整套路由上。
///
/// 与 `with_project_repository` 同一种形态——只加一层 `Extension`，
/// 不新增路由，也不绕过已经装好的门禁。缺这一步健康页一律回 404。
pub fn with_health_evidence(app: Router, evidence: Arc<dyn HealthEvidence>) -> Router {
    app.layer(Extension(HealthEvidenceSource(evidence)))
}

/// 未装配证据来源时的固定响应体。
#[derive(serde::Serialize)]
struct Absent {
    error: &'static str,
}

async fn page(evidence: Option<Extension<HealthEvidenceSource>>) -> Response {
    // 能走到这里说明会话门禁那一半在装配期已成立；证据来源那一半在这里复核。
    let Some(Extension(HealthEvidenceSource(source))) = evidence else {
        return (
            StatusCode::NOT_FOUND,
            Json(Absent {
                error: "本机未装配健康证据来源。",
            }),
        )
            .into_response();
    };
    // 取证据是纯读；即使实现内部发生阻塞也不改变任何状态。
    let snapshot = tokio::task::spawn_blocking(move || source.snapshot())
        .await
        .unwrap_or_else(|_| EvidenceSnapshot::unknown(0));
    Json(HealthBody::from_report(&snapshot.report())).into_response()
}

#[cfg(test)]
mod tests;
