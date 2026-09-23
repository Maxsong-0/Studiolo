//! 人侧项目结构查询；文档使用完整代次，仓库目录单独记录当前观测时间。

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use studiolo_workbench::{DirectoryDrift, ProjectArchitecture, ProjectOperations};

use crate::{
    ApiError, AppState, ProjectRepository, project_cargo::CargoDependencyObservation,
    project_repository::RepositoryCheck, project_rust::RustCallObservation, queries,
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
    rust_call_sites: RustCallObservation,
}

pub(crate) async fn get(
    State(state): State<AppState>,
    repository: Option<Extension<Arc<ProjectRepository>>>,
    query: Result<Query<OperationQuery>, QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return crate::invalid_query("invalid_operations_query");
    };
    operations_at(state, repository, query)
        .await
        .into_response()
}

async fn operations_at(
    state: AppState,
    repository: Option<Extension<Arc<ProjectRepository>>>,
    query: OperationQuery,
) -> Result<Json<OperationResponse>, ApiError> {
    let generation = queries::at_generation(&state, query.generation)?;
    let session_mode = state.sessions.is_some();
    queries::run_query(&state.queries, move || {
        let mut operations = generation
            .model
            .project_operations(&query.project)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "项目不在当前只读快照中。".into()))?;
        let architecture = generation
            .model
            .project_architecture(&query.project)
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "项目不在当前只读快照中。".into()))?;
        let binding = repository
            .as_ref()
            .map(|Extension(repository)| repository)
            .filter(|repository| {
                repository.matches(&query.project)
                    && (!session_mode || repository.approved_for_sessions())
            });
        let mut cargo_dependencies = match binding {
            Some(repository) => repository.observe_cargo_dependencies(),
            _ => CargoDependencyObservation::unbound(),
        };
        let mut rust_call_sites = match binding {
            Some(repository) => repository.observe_rust_call_sites(),
            _ => RustCallObservation::unbound(),
        };
        let mut repository = match binding {
            Some(repository) => {
                if operations.declaration_status == "ready" {
                    repository.observe(&operations.comparison_parents)
                } else {
                    RepositoryCheck::unavailable()
                }
            }
            _ => RepositoryCheck::unbound(),
        };
        if let Some(binding) = binding {
            finalize_approval(
                binding,
                &mut repository,
                &mut cargo_dependencies,
                &mut rust_call_sites,
            );
        }
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
            rust_call_sites,
        }))
    })
    .await
}

/// 批准对象是整组证据的共同前提；后续目录检查失败不能留下先前成功的 Cargo 图。
fn finalize_approval(
    binding: &ProjectRepository,
    repository: &mut RepositoryCheck,
    cargo: &mut CargoDependencyObservation,
    rust: &mut RustCallObservation,
) {
    if binding.approval_is_current() {
        return;
    }
    *repository = RepositoryCheck::unavailable();
    cargo.graph = None;
    cargo.observed_at = None;
    rust.graph = None;
    rust.links = None;
    rust.observed_at = None;
    // 未授权能力仍为 disabled；授权漂移不会反向变成启用状态。
    for (status, diagnostics) in [
        (&mut cargo.status, &mut cargo.diagnostics),
        (&mut rust.status, &mut rust.diagnostics),
    ] {
        if *status != "disabled" {
            *status = "unavailable";
            *diagnostics = vec!["私有仓库批准或服务根已变化，本次仓库证据全部不可用。".into()];
        }
    }
}

#[cfg(test)]
#[path = "../tests/repository_approval/mod.rs"]
mod approval_contract;
