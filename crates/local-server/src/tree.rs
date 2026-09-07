//! 有界文件树查询：只访问当前代次内存，不重新读取源路径。

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use studiolo_workbench::{TreeError, TreePage};

use crate::{ApiError, AppState, queries};

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    path: String,
    #[serde(default)]
    offset: usize,
    generation: u64,
}

#[derive(Serialize)]
pub(crate) struct TreeResponse {
    generation: u64,
    #[serde(flatten)]
    page: TreePage,
}

pub(crate) async fn page(
    State(state): State<AppState>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, ApiError> {
    let generation = queries::at_generation(&state, query.generation)?;
    queries::run_query(&state.queries, move || {
        let page =
            generation
                .tree
                .page(&query.path, query.offset)
                .map_err(|error| match error {
                    TreeError::NotFound => {
                        ApiError(StatusCode::NOT_FOUND, "目录不在当前只读快照中。".into())
                    }
                    TreeError::InvalidPath | TreeError::NotDirectory | TreeError::InvalidOffset => {
                        ApiError(StatusCode::BAD_REQUEST, "目录路径或分页位置无效。".into())
                    }
                    TreeError::InvalidInput => ApiError::unavailable(),
                })?;
        Ok(Json(TreeResponse {
            generation: generation.number,
            page,
        }))
    })
    .await
}
