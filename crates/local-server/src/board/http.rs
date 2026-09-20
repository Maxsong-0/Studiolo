use super::{BoardPlane, requests};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Query, Request},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;

pub(super) fn routes(routes: Router<crate::AppState>) -> Router<crate::AppState> {
    routes
        .route("/api/v1/board", get(view))
        .route("/api/v1/board/replay", get(super::replay::replay))
        .route("/api/v1/board/timeline", get(super::timeline::timeline))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryProject {
    project: String,
}
async fn view(
    plane: Option<Extension<Arc<BoardPlane>>>,
    Query(q): Query<QueryProject>,
) -> Response {
    let Some(Extension(plane)) = plane else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if studiolo_core::VaultPath::parse(&q.project).is_err() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.storage.check().map_err(str::to_owned)?;
        let hash = plane
            .policy
            .effective()
            .map_err(|_| "策略不可用".to_owned())?
            .effective_hash;
        let engine = plane.engine.lock().map_err(|_| "黑板不可用".to_owned())?;
        let items = engine
            .board_list(&q.project, &hash)
            .map_err(|e| e.to_string())?;
        items
            .into_iter()
            .map(|item| {
                let mut lineages = Vec::new();
                for id in [
                    item.board.created_by_instance.as_deref(),
                    item.holder.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<std::collections::BTreeSet<_>>()
                {
                    let lineage = plane
                        .identities
                        .instance_lineage(id)
                        .map_err(|_| "实例链不可用")?;
                    lineages.push(
                        lineage
                            .iter()
                            .rev()
                            .map(|instance| {
                                serde_json::json!({
                                    "id": instance.id, "agent_type": instance.agent_type,
                                })
                            })
                            .collect::<Vec<_>>(),
                    );
                }
                let mut value = serde_json::to_value(item).map_err(|_| "黑板编码失败")?;
                value["instance_lineages"] = serde_json::json!(lineages);
                Ok::<_, String>(value)
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await;
    match result {
        Ok(Ok(items)) => Json(items).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
pub(super) fn private_router(plane: Arc<BoardPlane>) -> Router {
    Router::new()
        .route("/internal/board", post(command))
        .layer(DefaultBodyLimit::max(262_144))
        .layer(Extension(plane))
        .layer(middleware::from_fn(peer))
}
async fn peer(request: Request, next: middleware::Next) -> Response {
    if crate::unix_ingress::source(&request).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}
async fn command(
    Extension(plane): Extension<Arc<BoardPlane>>,
    Json(request): Json<requests::Request>,
) -> Response {
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.execute(request)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(reason)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":reason})),
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
