//! 同代次的首屏、搜索与导航；CPU 工作受并发上限控制，不占用 HTTP 运行线程。
//!
//! 本机入口有两道互不替代的准入，刻意不合并成一道：
//!
//! - **查询许可**（本文件的 `queries` 信号量）限制同时占用的后台工作槽。它管的是
//!   「同时有多少读侧 CPU 工作在跑」，因此首屏、搜索、原文、文件树、项目运作一律
//!   走它，读侧不再有绕开计数的路径。
//! - **刷新闸门**（`crate::refresh`）限制的是「同一批并发刷新最多换来几次派生」，
//!   靠合流而不是拒绝：并发刷新排到同一张票上等同一次派生，没有请求因为别人正在
//!   刷新而被拒。派生本身不占查询许可——它已经被闸门硬限制成至多一次在途，再去挤
//!   工作槽只会把搜索饿死；反过来把首屏放出查询许可，读侧计数就出现缺口。
//!
//! 两者的交接点在首屏：刷新合流之后，同一批请求会同时来投影同一代次。实测九个并发
//! `?refresh=true` 在 `try_acquire` 下只有两个拿到工作槽，其余七个拿到 429——一次共享
//! 派生被合流省下来，却在最后一步变成随机失败。所以首屏改用 [`run_entry_query`]：
//! 满额时排队等待而不是快速失败。搜索、原文、文件树这些页内查询仍旧快速失败，它们
//! 有明确的重试位置；首屏是整个界面的入口，没有可退回的上一屏。
//!
//! 静态资源（`crate::assets`）两者都不占：它既不读代次，也不做随文档量增长的 CPU
//! 工作，只是有上限的单文件读取，占用工作槽只会把前端外壳和真实查询互相拖住。

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
    WorkbenchOverview,
};

use crate::{ApiError, AppState, Generation, current, view::Freshness};

/// 首屏响应：项目、收件箱、诊断与计数；逐篇文档走文件树分页与搜索，不在此整份下发。
#[derive(Serialize)]
pub(crate) struct OverviewResponse {
    generation: u64,
    source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    freshness: Option<Freshness>,
    #[serde(flatten)]
    overview: WorkbenchOverview,
}

/// 读取当前代次并投影首屏；投影与序列化都在查询许可内的后台线程上完成。
pub(crate) async fn overview(state: AppState) -> Result<Json<OverviewResponse>, ApiError> {
    let view = state
        .current
        .snapshot()
        .map_err(|_| ApiError::unavailable())?;
    let generation = view.generation;
    let freshness = view.freshness;
    let source_label = state.source_label.clone();
    run_entry_query(&state.queries, move || {
        Ok(Json(OverviewResponse {
            generation: generation.number,
            source_label,
            freshness,
            overview: generation.model.overview(),
        }))
    })
    .await
}

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
    on_worker(query_permit(queries)?, work).await
}

/// 首屏专用：满额时排队等待工作槽，而不是把入口请求快速失败掉。
///
/// 等待者是一个廉价 future，客户端断开就直接消失，不会占住工作槽；真正占住工作槽的
/// 始终只有已经开始跑的那两个查询。
pub(crate) async fn run_entry_query<T: Send + 'static>(
    queries: &Arc<tokio::sync::Semaphore>,
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = queries
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::unavailable())?;
    on_worker(permit, work).await
}

async fn on_worker<T: Send + 'static>(
    permit: tokio::sync::OwnedSemaphorePermit,
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
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
    async fn the_first_screen_waits_for_a_work_slot_while_page_queries_still_fail_fast() {
        let budget = Arc::new(tokio::sync::Semaphore::new(1));
        let held = budget.clone().acquire_owned().await.ok().unwrap();
        let entry_budget = Arc::clone(&budget);
        let entry = tokio::spawn(async move { run_entry_query(&entry_budget, || Ok(7)).await });
        tokio::task::yield_now().await;
        assert!(matches!(
            query_permit(&budget),
            Err(ApiError(StatusCode::TOO_MANY_REQUESTS, _))
        ));
        assert!(!entry.is_finished(), "首屏在额度满时排队等待而不是失败");
        drop(held);
        assert!(matches!(entry.await, Ok(Ok(7))));
        assert_eq!(budget.available_permits(), 1);
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
