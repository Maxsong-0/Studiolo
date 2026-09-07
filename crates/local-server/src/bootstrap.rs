//! 仅供前端选择认证流程的固定模式，不返回内容或产生授权。

use axum::{Json, extract::State};
use serde::Serialize;

use crate::AppState;

pub(crate) const PATH: &str = "/api/v1/bootstrap";

#[derive(Serialize)]
pub(crate) struct Bootstrap {
    authentication: &'static str,
}

pub(crate) async fn mode(State(state): State<AppState>) -> Json<Bootstrap> {
    Json(Bootstrap {
        authentication: if state.sessions.is_some() {
            "session"
        } else if state.access.is_some() {
            "access"
        } else {
            "local"
        },
    })
}
