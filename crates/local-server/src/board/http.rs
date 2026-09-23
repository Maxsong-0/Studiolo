use super::{BoardPlane, requests};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Query, Request, rejection::QueryRejection},
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
    query: Result<Query<QueryProject>, QueryRejection>,
) -> Response {
    // 查询串拒绝仍先于平面检查，状态码与原提取器一致，只把正文换成固定中文 JSON。
    let Ok(Query(q)) = query else {
        return crate::invalid_query("invalid_board_query");
    };
    let Some(Extension(plane)) = plane else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // 项目路径不合法同属畸形查询串：同一答复，不回显字段名。
    if studiolo_core::VaultPath::parse(&q.project).is_err() {
        return crate::invalid_query("invalid_board_query");
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

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header::CONTENT_TYPE},
        routing::get,
    };
    use tower::ServiceExt;

    /// 不装黑板平面也能核对查询串拒绝：拒绝先于平面检查，与原提取器的状态码一致。
    fn app() -> Router {
        Router::new()
            .route("/api/v1/board", get(super::view))
            .route("/api/v1/board/replay", get(super::super::replay::replay))
            .route(
                "/api/v1/board/timeline",
                get(super::super::timeline::timeline),
            )
    }

    async fn reply(uri: &str) -> (StatusCode, String, String) {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let response = app().oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        (
            status,
            content_type,
            String::from_utf8(body.to_vec()).unwrap(),
        )
    }

    #[tokio::test]
    async fn malformed_board_queries_answer_chinese_json_without_field_names() {
        for (uri, code) in [
            ("/api/v1/board", "invalid_board_query"),
            ("/api/v1/board?project=a&extra=1", "invalid_board_query"),
            (
                "/api/v1/board/replay?task_id=not-a-task",
                "invalid_replay_query",
            ),
            (
                "/api/v1/board/replay?task_id=x&after=-1",
                "invalid_replay_query",
            ),
            ("/api/v1/board/timeline?project=a", "invalid_timeline_query"),
        ] {
            let (status, content_type, body) = reply(uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(content_type, "application/json", "{uri}");
            let value: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(value["code"], code);
            assert_eq!(value["error"], "查询参数不合法。");
            for leaked in [
                "project", "task_id", "after", "extra", "from", "field", "Failed",
            ] {
                assert!(!body.contains(leaked), "{uri} 回显了 {leaked}：{body}");
            }
        }
        // 查询合法而平面未装：仍是 404，不因拒绝改造而变。
        assert_eq!(
            reply("/api/v1/board?project=a").await.0,
            StatusCode::NOT_FOUND
        );
    }
}
