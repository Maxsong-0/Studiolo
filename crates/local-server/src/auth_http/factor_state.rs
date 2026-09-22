//! 只返回经过当前凭据检查的显示快照，不把能力缺失与运行故障混为一谈。

use axum::{
    Json,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::sync::Arc;
use studiolo_auth::{FactorState, RecentAuthentication};

use super::{Context, responses::Failure};
use crate::AppState;

pub(super) async fn handle(State(state): State<AppState>, mut request: Request) -> Response {
    let result = async move {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(Failure::unavailable)?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable())?;
        let credentials = context.credentials.ok_or_else(Failure::unavailable)?;
        let auth = state.sessions.as_ref().ok_or_else(Failure::unavailable)?;
        let factors = if let Some(factors) = &auth.factors {
            let factors = Arc::clone(factors);
            Some(
                auth.blocking(move || {
                    factors.state(&context.identity, &credentials.token, &credentials.csrf)
                })
                .await
                .map_err(|blocked| blocked.session().for_read())?
                .into(),
            )
        } else {
            // 此null只来自明确组合；读取失败绝不能伪装成未启用。
            None
        };
        Ok(Json(PublicState { factors }).into_response())
    }
    .await;
    result.unwrap_or_else(Failure::response)
}

#[derive(Serialize)]
struct PublicState {
    factors: Option<PublicFactor>,
}

#[derive(Serialize)]
struct PublicFactor {
    observed_at_ms: i64,
    recent_authentication: Option<PublicRecent>,
    recovery_candidate_available: bool,
    pending: bool,
}

#[derive(Serialize)]
struct PublicRecent {
    verified_at_ms: i64,
    expires_at_ms: i64,
}

impl From<FactorState> for PublicFactor {
    fn from(value: FactorState) -> Self {
        Self {
            observed_at_ms: value.observed_at_ms,
            recent_authentication: value.recent_authentication.map(PublicRecent::from),
            recovery_candidate_available: value.recovery_candidate_available,
            pending: value.pending,
        }
    }
}

impl From<RecentAuthentication> for PublicRecent {
    fn from(value: RecentAuthentication) -> Self {
        Self {
            verified_at_ms: value.verified_at_ms,
            expires_at_ms: value.expires_at_ms,
        }
    }
}
