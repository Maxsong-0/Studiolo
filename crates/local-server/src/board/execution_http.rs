//! 人侧执行控制：敏感副作用发生之前，先核当前会话、落持久审计、过 L3。
use super::{BoardPlane, RunnerPlane, StartRequest};
use crate::{
    AppState,
    auth_http::execution::{self as input, Input, Principal},
    editing::Problem,
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, AuditSubject, SanitizedPayload};
use studiolo_auth::SensitiveAction;
use studiolo_core::TaskId;
use studiolo_task_engine::TaskEngine;

pub(super) fn routes(routes: Router<AppState>) -> Router<AppState> {
    routes
        .route(input::STATUS, get(status))
        .route(input::COMMAND, post(command))
}
fn problem(status: StatusCode, reason: impl Into<String>) -> Response {
    (status, Json(json!({"error":reason.into()}))).into_response()
}
/// 未装配黑板平面或会话门禁：仍是 404，正文从空换成固定中文 JSON。
fn unconfigured() -> Response {
    Problem::new(
        StatusCode::NOT_FOUND,
        "execution_unconfigured",
        "执行控制未配置。",
    )
    .into_response()
}
async fn status(request: Request) -> Response {
    let Some(runner) = request.extensions().get::<Arc<RunnerPlane>>().cloned() else {
        return Json(json!({"configured":false,"frozen":true,"runs":[]})).into_response();
    };
    match tokio::task::spawn_blocking(move || {
        Ok::<_, String>(
            json!({"configured":true,"frozen":runner.is_frozen(),"runs":runner.replay()?}),
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => problem(StatusCode::SERVICE_UNAVAILABLE, "执行体状态不可用"),
    }
}
async fn command(State(state): State<AppState>, mut request: Request) -> Response {
    let Some(board) = request.extensions().get::<Arc<BoardPlane>>().cloned() else {
        return unconfigured();
    };
    let runner = request.extensions().get::<Arc<RunnerPlane>>().cloned();
    let Some(auth) = &state.sessions else {
        return unconfigured();
    };
    let (principal, input) = match auth.execution(&mut request) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let Some(input) = input else {
        return Problem::new(
            StatusCode::BAD_REQUEST,
            "invalid_execution_request",
            "执行请求不合法。",
        )
        .into_response();
    };
    let permit = match board.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return super::http::busy(),
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        execute(&board, runner.as_deref(), &principal, input)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(reason)) => problem(StatusCode::CONFLICT, reason),
        Err(_) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "执行结果尚不能确认，请刷新状态",
        ),
    }
}
fn event(
    board: &BoardPlane,
    id: &str,
    action: &str,
    task: Option<TaskId>,
    phase: AuditPhase,
    reason: &str,
) -> Result<(), String> {
    let mut subject = AuditSubject::default();
    if let Some(task) = task {
        subject = subject.with_task(task);
    }
    let event = AuditEvent::new(
        AuditEventId::parse(id).map_err(|_| "审计编号失败")?,
        phase,
        action,
        SanitizedPayload::try_from(json!({"reason":reason})).map_err(|_| "审计编码失败")?,
    )
    .with_subject(subject);
    board
        .audit
        .append(event)
        .map_err(|_| "权威审计不可用；结果需核验")?;
    Ok(())
}
fn execute(
    board: &BoardPlane,
    runner: Option<&RunnerPlane>,
    principal: &Principal,
    input: Input,
) -> Result<Value, String> {
    principal.verify()?;
    board.storage.check().map_err(str::to_owned)?;
    let (action, task) = match &input {
        Input::Freeze { frozen: true } => ("execution.freeze", None),
        Input::Freeze { frozen: false } => ("execution.unfreeze", None),
        Input::Start { task_id, .. } => ("execution.start", Some(*task_id)),
        Input::Cancel { task_id } => ("execution.cancel", Some(*task_id)),
        Input::Review { task_id, .. } => ("execution.review", Some(*task_id)),
        Input::Land { task_id, .. } => ("execution.land", Some(*task_id)),
    };
    let id = format!("execution:{}", ulid::Ulid::generate());
    event(board, &id, action, task, AuditPhase::Intent, "requested")?;
    let result = (|| match input {
        Input::Freeze { frozen } => {
            if !frozen {
                principal.sensitive(SensitiveAction::ReleaseFreeze)?;
            }
            runner.ok_or("未配置真实执行体")?.freeze(frozen)?;
            Ok(json!({"frozen":frozen}))
        }
        Input::Start {
            task_id,
            agent_type,
            instance_id,
        } => super::execute::value(runner.ok_or("未配置真实执行体")?.start_task(StartRequest {
            task_id,
            agent_type,
            instance_id,
        })?),
        Input::Cancel { task_id } => {
            runner.ok_or("未配置真实执行体")?.cancel(task_id)?;
            Ok(json!({"ok":true}))
        }
        Input::Land {
            task_id,
            expected_head,
            patch_sha256,
        } => {
            principal.verify()?;
            principal.sensitive(SensitiveAction::ApproveCandidate)?;
            super::execute::value(runner.ok_or("未配置真实执行体")?.land_candidate(
                task_id,
                expected_head,
                patch_sha256,
            )?)
        }
        Input::Review { task_id, receipt } => {
            // 整个终态审阅事务期间都持有仓库落地权。
            let code = {
                let engine = board.engine.lock().map_err(|_| "黑板不可用")?;
                engine
                    .task(task_id)
                    .map_err(|_| "工作项不可用")?
                    .is_some_and(|t| matches!(t.spec.adapter.as_str(), "claude" | "codex"))
            };
            let _landing =
                if code && receipt.status == studiolo_task_engine::ReceiptStatus::Completed {
                    Some(
                        runner
                            .ok_or("未配置真实执行体")?
                            .require_landed_candidate(task_id)?,
                    )
                } else {
                    None
                };
            let mut engine = board.engine.lock().map_err(|_| "黑板不可用")?;
            principal.verify()?;
            principal.sensitive(SensitiveAction::ApproveCandidate)?;
            let reviewed = engine
                .review_board_candidates(task_id, receipt, jiff::Timestamp::now().as_millisecond())
                .map_err(|e| e.to_string())?;
            let hash = board
                .policy
                .effective()
                .map_err(|_| "验收已记录，策略状态暂不可用")?
                .effective_hash;
            board.project(&mut engine, &reviewed.task.spec.project, &hash)?;
            super::execute::value(
                engine
                    .board_detail(task_id, &hash)
                    .map_err(|e| e.to_string())?,
            )
        }
    })();
    event(
        board,
        &id,
        action,
        task,
        AuditPhase::Outcome,
        if result.is_ok() {
            "completed"
        } else {
            "rejected_or_incomplete"
        },
    )?;
    result
}
