//! 同代次的搜索与导航；CPU 工作受并发上限控制，不占用 HTTP 运行线程。

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use studiolo_core::VaultPath;
use studiolo_workbench::{
    DocumentNavigation, DocumentView, SearchError, SearchMode, SearchQuery, SearchResults,
};

use crate::{ApiError, AppState, Generation, current};

#[derive(Deserialize)]
pub(crate) struct QueryParameters {
    q: String,
    mode: SearchMode,
    project: Option<String>,
    generation: u64,
}

#[derive(Serialize)]
pub(crate) struct SearchResponse {
    generation: u64,
    #[serde(flatten)]
    results: SearchResults,
}

pub(crate) async fn search(
    State(state): State<AppState>,
    Query(query): Query<QueryParameters>,
) -> Result<Json<SearchResponse>, ApiError> {
    let generation = at_generation(&state, query.generation)?;
    run_query(&state.queries, move || {
        let results = generation
            .model
            .search(&SearchQuery {
                text: query.q,
                project: query.project,
                mode: query.mode,
            })
            .map_err(|error| match error {
                SearchError::InvalidQuery => ApiError(
                    StatusCode::BAD_REQUEST,
                    "查询须为不含控制字符的短文本（最多 256 字符）。".into(),
                ),
                SearchError::InvalidProject => {
                    ApiError(StatusCode::BAD_REQUEST, "项目不在当前只读快照中。".into())
                }
                SearchError::Engine(_) => ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "搜索暂不可用，请调整查询后重试。".into(),
                ),
            })?;
        Ok(Json(SearchResponse {
            generation: generation.number,
            results,
        }))
    })
    .await
}

#[derive(Deserialize)]
pub(crate) struct DocumentQuery {
    path: String,
    generation: u64,
}

#[derive(Serialize)]
pub(crate) struct DocumentResponse {
    #[serde(flatten)]
    document: DocumentView,
    navigation: DocumentNavigation,
}

pub(crate) async fn document(
    State(state): State<AppState>,
    Query(query): Query<DocumentQuery>,
) -> Result<Json<DocumentResponse>, ApiError> {
    let generation = at_generation(&state, query.generation)?;
    let path = VaultPath::parse(&query.path)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "无效的文档路径。".into()))?;
    run_query(&state.queries, move || {
        let document = generation.model.document(&path).ok_or_else(not_found)?;
        let navigation = generation.model.navigation(&path).ok_or_else(not_found)?;
        Ok(Json(DocumentResponse {
            document,
            navigation,
        }))
    })
    .await
}

pub(crate) async fn run_query<T: Send + 'static>(
    queries: &Arc<tokio::sync::Semaphore>,
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = query_permit(queries)?;
    tokio::task::spawn_blocking(move || {
        // 客户端取消也不会提早释放额度；此工作真正结束后才释放。
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| ApiError::unavailable())?
}

pub(crate) fn at_generation(state: &AppState, number: u64) -> Result<Arc<Generation>, ApiError> {
    let generation = current(state)?;
    if generation.number != number {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "视图已刷新，请刷新工作台后重试。".into(),
        ));
    }
    Ok(generation)
}

fn query_permit(
    queries: &Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
    queries.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "正在处理其他查询，请稍后重试。".into(),
        )
    })
}

fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "文档不在当前只读快照中。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_queries_fail_fast_and_capacity_returns_only_after_work_finishes() {
        let budget = Arc::new(tokio::sync::Semaphore::new(2));
        let first = query_permit(&budget).ok().unwrap();
        let second = query_permit(&budget).ok().unwrap();
        assert!(matches!(
            query_permit(&budget),
            Err(ApiError(StatusCode::TOO_MANY_REQUESTS, _))
        ));
        drop(first);
        let next = query_permit(&budget).ok().unwrap();
        assert_eq!(budget.available_permits(), 0);
        drop((next, second));
        assert_eq!(budget.available_permits(), 2);
    }

    #[tokio::test]
    async fn cancelling_a_request_does_not_release_its_running_query_slot() {
        let budget = Arc::new(tokio::sync::Semaphore::new(1));
        let request_budget = Arc::clone(&budget);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let request = tokio::spawn(async move {
            run_query(&request_budget, move || {
                let _ = started_tx.send(());
                // 发送端因断言失败而被丢弃时也退出，不留下卡住的测试线程。
                let _ = release_rx.recv();
                Ok(())
            })
            .await
        });
        started_rx.await.unwrap();
        request.abort();
        assert!(matches!(request.await, Err(error) if error.is_cancelled()));
        assert_eq!(budget.available_permits(), 0);
        assert!(matches!(
            query_permit(&budget),
            Err(ApiError(StatusCode::TOO_MANY_REQUESTS, _))
        ));
        release_tx.send(()).unwrap();
        let returned = budget.clone().acquire_owned().await.unwrap();
        drop(returned);
        assert_eq!(budget.available_permits(), 1);
    }
}
