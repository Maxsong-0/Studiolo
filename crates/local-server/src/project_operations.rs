//! 人侧项目结构查询；文档使用完整代次，仓库目录单独记录当前观测时间。

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use studiolo_workbench::{DirectoryDrift, ProjectArchitecture, ProjectOperations};

use crate::{
    ApiError, AppState, ProjectRepository, project_cargo::CargoDependencyObservation,
    project_repository::RepositoryCheck, queries,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperationQuery {
    project: String,
    generation: u64,
}

#[derive(Serialize)]
pub(crate) struct OperationResponse {
    generation: u64,
    #[serde(flatten)]
    operations: ProjectOperations,
    repository: RepositoryCheck,
    drift: Vec<DirectoryDrift>,
    architecture: ProjectArchitecture,
    cargo_dependencies: CargoDependencyObservation,
}

pub(crate) async fn get(
    State(state): State<AppState>,
    repository: Option<Extension<Arc<ProjectRepository>>>,
    Query(query): Query<OperationQuery>,
) -> Result<Json<OperationResponse>, ApiError> {
    let generation = queries::at_generation(&state, query.generation)?;
    queries::run_query(&state.queries, move || {
        let mut operations = generation
            .model
            .project_operations(&query.project)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "项目不在当前只读快照中。".into()))?;
        let architecture = generation
            .model
            .project_architecture(&query.project)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "项目不在当前只读快照中。".into()))?;
        let cargo_dependencies = match &repository {
            Some(Extension(repository)) if repository.matches(&query.project) => {
                repository.observe_cargo_dependencies()
            }
            _ => CargoDependencyObservation::unbound(),
        };
        let repository = match repository {
            Some(Extension(repository)) if repository.matches(&query.project) => {
                if operations.declaration_status == "ready" {
                    repository.observe(&operations.comparison_parents)
                } else {
                    RepositoryCheck::unavailable()
                }
            }
            _ => RepositoryCheck::unbound(),
        };
        if repository.status == "unavailable" && operations.declaration_status == "ready" {
            operations.diagnostics.push(
                "仓库观测不完整、超出预算或发生变化；本次不判定目录漂移，请重新读取。".into(),
            );
        }
        let drift = operations.compare(&repository.observations);
        Ok(Json(OperationResponse {
            generation: generation.number,
            operations,
            repository,
            drift,
            architecture,
            cargo_dependencies,
        }))
    })
    .await
}
