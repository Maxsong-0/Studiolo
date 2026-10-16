use super::{BoardPlane, execute::Failure, requests};
use crate::editing::Problem;
use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Query, Request,
        rejection::{JsonRejection, QueryRejection},
    },
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
        .route("/api/v1/board/overview", get(super::overview::overview))
        .route("/api/v1/board/replay", get(super::replay::replay))
        .route("/api/v1/board/timeline", get(super::timeline::timeline))
}
/// 未装配黑板平面：仍是 404，正文是固定中文 JSON；只读监督路由（含跨项目汇总 overview）共用。
pub(super) fn unconfigured() -> Response {
    Problem::new(StatusCode::NOT_FOUND, "board_unconfigured", "黑板未配置。").into_response()
}
/// 黑板许可已满：仍是 429，本次请求没有进入控制库；人侧与私有入口共用。
pub(super) fn busy() -> Response {
    Problem::new(
        StatusCode::TOO_MANY_REQUESTS,
        "board_busy",
        "黑板繁忙，请稍后重试。",
    )
    .into_response()
}
/// 控制存储、身份库、策略、审计或阻塞任务失败：仍是 503，只给固定句子，不回显内部原因。
pub(super) fn unavailable() -> Response {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "board_unavailable",
        "黑板暂不可用，请重试。",
    )
    .into_response()
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
        return unconfigured();
    };
    // 项目路径不合法同属畸形查询串：同一答复，不回显字段名。
    if studiolo_core::VaultPath::parse(&q.project).is_err() {
        return crate::invalid_query("invalid_board_query");
    }
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return busy(),
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
        _ => unavailable(),
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
        return Problem::new(
            StatusCode::FORBIDDEN,
            "board_peer_forbidden",
            "只接受本机受信连接。",
        )
        .into_response();
    }
    next.run(request).await
}
async fn command(
    Extension(plane): Extension<Arc<BoardPlane>>,
    request: Result<Json<requests::Request>, JsonRejection>,
) -> Response {
    // 正文拒绝沿用原提取器的状态码（语法错 400、缺 JSON 类型 415、字段或取值不符 422、
    // 超长 413），正文换成固定中文 JSON：agent 侧按 JSON 取 error，不再退化成「黑板响应无效」。
    let request = match request {
        Ok(Json(request)) => request,
        Err(rejection) => {
            return Problem::new(
                rejection.status(),
                "invalid_board_request",
                "黑板请求不合法。",
            )
            .into_response();
        }
    };
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return busy(),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.execute(request)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(Failure::Rejected(reason))) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":reason})),
        )
            .into_response(),
        // 授权这一步读不出身份库、策略库或控制存储：结论未知、什么都没写，按暂不可用答复，
        // 不能回 409 让调用方当成被拒（BUG-032）。具体原因只进审计。
        Ok(Err(Failure::Unavailable)) | Err(_) => unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header::CONTENT_TYPE},
        response::Response,
        routing::get,
    };
    use serde_json::{Value, json};
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

    async fn parts(response: Response) -> (StatusCode, String, String) {
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

    async fn reply(uri: &str) -> (StatusCode, String, String) {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        parts(app().oneshot(request).await.unwrap()).await
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
            let value: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(value["code"], code);
            assert_eq!(value["error"], "查询参数不合法。");
            for leaked in [
                "project", "task_id", "after", "extra", "from", "field", "Failed",
            ] {
                assert!(!body.contains(leaked), "{uri} 回显了 {leaked}：{body}");
            }
        }
        // 查询合法而平面未装：仍是 404，正文从空换成固定中文 JSON。
        for uri in [
            "/api/v1/board?project=a",
            "/api/v1/board/replay?task_id=01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "/api/v1/board/timeline?project=a&from=2026-09-01T00:00:00Z&to=2026-09-02T00:00:00Z",
        ] {
            let (status, content_type, body) = reply(uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(content_type, "application/json", "{uri}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({"code": "board_unconfigured", "error": "黑板未配置。"}),
                "{uri}"
            );
        }
    }

    /// 429 要占满真实黑板平面的许可，而平面只能由带文件夹具的集成测试构造；这里钉住 429 与
    /// 503 答复本身的线上形态（404／403／503 的真实触发见 tests/rejection_bodies_contract.rs）。
    #[tokio::test]
    async fn busy_and_unavailable_keep_their_status_with_fixed_chinese_json() {
        for (response, status, code, error) in [
            (
                super::busy(),
                StatusCode::TOO_MANY_REQUESTS,
                "board_busy",
                "黑板繁忙，请稍后重试。",
            ),
            (
                super::unavailable(),
                StatusCode::SERVICE_UNAVAILABLE,
                "board_unavailable",
                "黑板暂不可用，请重试。",
            ),
        ] {
            let (got, content_type, body) = parts(response).await;
            assert_eq!(got, status, "{code}");
            assert_eq!(content_type, "application/json", "{code}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({"code": code, "error": error})
            );
        }
    }
}
