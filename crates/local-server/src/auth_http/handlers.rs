//! 三条封闭路径的交付；变更动作在库中重新检查，不把前置快照作为持续授权。

use std::sync::Arc;

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use super::{
    AuthHttp, Context, cookie,
    input::{Action, Revoke},
    responses::{self, Failure, PublicSession},
};
use crate::AppState;

fn context(request: &Request) -> Result<Arc<Context>, Failure> {
    request
        .extensions()
        .get::<Arc<Context>>()
        .cloned()
        .ok_or_else(Failure::unavailable)
}

fn auth(state: &AppState) -> Result<AuthHttp, Failure> {
    state.sessions.clone().ok_or_else(Failure::unavailable)
}

pub(super) async fn current(request: Request) -> Response {
    let result = (|| {
        let context = context(&request)?;
        let info = context.session.as_ref().ok_or_else(Failure::unavailable)?;
        let credentials = context
            .credentials
            .as_ref()
            .ok_or_else(Failure::unavailable)?;
        Ok(responses::session(
            info,
            credentials.csrf.expose_secret(),
            StatusCode::OK,
        ))
    })();
    result.unwrap_or_else(Failure::response)
}

pub(super) async fn list(State(state): State<AppState>, request: Request) -> Response {
    let result = async move {
        let context = context(&request)?;
        let credentials = Arc::clone(
            context
                .credentials
                .as_ref()
                .ok_or_else(Failure::unavailable)?,
        );
        let auth = auth(&state)?;
        let rows = auth
            .run(move |sessions| sessions.sessions(&context.identity, &credentials.token))
            .await?;
        #[derive(Serialize)]
        struct Body<'a> {
            sessions: Vec<PublicSession<'a>>,
        }
        Ok(Json(Body {
            sessions: rows.iter().map(PublicSession::from).collect(),
        })
        .into_response())
    }
    .await;
    result.unwrap_or_else(Failure::response)
}

pub(super) async fn issue(State(state): State<AppState>, request: Request) -> Response {
    let result = async move {
        let context = context(&request)?;
        if !matches!(context.action, Some(Action::Issue)) || context.credentials.is_some() {
            return Err(Failure::unavailable());
        }
        let issued = auth(&state)?
            .run(move |sessions| sessions.issue_enrolled(&context.identity))
            .await?;
        let cookie = cookie::issued(&issued.token, &issued.csrf)?;
        let mut response = responses::session(
            &issued.session,
            issued.csrf.expose_secret(),
            StatusCode::CREATED,
        );
        response.headers_mut().insert(header::SET_COOKIE, cookie);
        Ok(response)
    }
    .await;
    result.unwrap_or_else(Failure::response)
}

pub(super) async fn revoke(State(state): State<AppState>, request: Request) -> Response {
    let result = async move {
        let context = context(&request)?;
        let credentials = Arc::clone(
            context
                .credentials
                .as_ref()
                .ok_or_else(Failure::unavailable)?,
        );
        let current = context.session.as_ref().ok_or_else(Failure::unavailable)?;
        let Some(Action::Revoke(action)) = &context.action else {
            return Err(Failure::unavailable());
        };
        let clear = match action {
            Revoke::Current | Revoke::All => true,
            Revoke::Session(target) => target == &current.id,
        };
        let result = auth(&state)?
            .run(move |sessions| {
                let Some(Action::Revoke(action)) = &context.action else {
                    return Err(studiolo_auth::SessionError::Rejected);
                };
                match action {
                    Revoke::Current => sessions.revoke_current(
                        &context.identity,
                        &credentials.token,
                        &credentials.csrf,
                    ),
                    Revoke::All => sessions.revoke_all(
                        &context.identity,
                        &credentials.token,
                        &credentials.csrf,
                    ),
                    Revoke::Session(target) => sessions.revoke(
                        &context.identity,
                        &credentials.token,
                        &credentials.csrf,
                        target,
                    ),
                }
            })
            .await?;
        #[derive(Serialize)]
        struct Body {
            revoked: usize,
        }
        let mut response = Json(Body {
            revoked: result.revoked,
        })
        .into_response();
        if clear {
            responses::clear_cookie(&mut response);
        }
        Ok(response)
    }
    .await;
    result.unwrap_or_else(Failure::response)
}
