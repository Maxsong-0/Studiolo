//! 三条因子动作只消费本次请求的秘密；响应失去连接不重放授权或秘密交付。

use std::sync::Arc;

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_auth::{RecentAuthentication, RecoveryCodes, TotpRequest};
use zeroize::Zeroizing;

use super::{
    Context, factor_input, input,
    responses::{self, Failure},
};
use crate::AppState;

enum Outcome {
    StepUp(RecentAuthentication),
    Codes(RecoveryCodes),
    Recovered,
}

pub(super) async fn handle(State(state): State<AppState>, mut request: Request) -> Response {
    let result = async move {
        // 该请求只有路由持有Context；不能为跨线程调用复制OTP或恢复码。
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(Failure::unavailable)?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable())?;
        let credentials = context.credentials.ok_or_else(Failure::unavailable)?;
        let source = context.source.ok_or_else(Failure::forbidden)?;
        let Some(input::Action::Factor(action)) = context.action else {
            return Err(Failure::unavailable());
        };
        let auth = state.sessions.as_ref().ok_or_else(Failure::unavailable)?;
        let outcome = auth
            .run_factor(move |factors| {
                let request = TotpRequest {
                    identity: &context.identity,
                    token: &credentials.token,
                    csrf: &credentials.csrf,
                    source: source.ip(),
                };
                match action {
                    factor_input::Action::StepUp(code) => {
                        factors.step_up(&request, code).map(Outcome::StepUp)
                    }
                    factor_input::Action::RotateCodes => {
                        factors.rotate_codes(&request).map(Outcome::Codes)
                    }
                    factor_input::Action::Recover {
                        recovery_code,
                        code,
                    } => factors
                        .recover(&request, recovery_code, code)
                        .map(|()| Outcome::Recovered),
                }
            })
            .await?;
        match outcome {
            Outcome::StepUp(value) => {
                #[derive(Serialize)]
                struct PublicRecent {
                    verified_at_ms: i64,
                    expires_at_ms: i64,
                }
                Ok(Json(PublicRecent {
                    verified_at_ms: value.verified_at_ms,
                    expires_at_ms: value.expires_at_ms,
                })
                .into_response())
            }
            Outcome::Codes(codes) => deliver_codes(codes),
            Outcome::Recovered => {
                let mut response = StatusCode::NO_CONTENT.into_response();
                responses::clear_cookie(&mut response);
                Ok(response)
            }
        }
    }
    .await;
    result.unwrap_or_else(Failure::response)
}

fn deliver_codes(codes: RecoveryCodes) -> Result<Response, Failure> {
    #[derive(Serialize)]
    struct Delivery<'a> {
        codes: Vec<&'a str>,
    }
    let delivery = Delivery {
        codes: codes
            .codes()
            .iter()
            .map(|code| code.expose_secret())
            .collect(),
    };
    // 十个固定55字节的规范原码在1KiB内；不复制成普通String或丢弃零化owner。
    let mut bytes = Zeroizing::new(Vec::with_capacity(1024));
    serde_json::to_writer(&mut *bytes, &delivery).map_err(|_| Failure::unavailable())?;
    let mut response = Response::new(Body::from(Bytes::from_owner(bytes)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}
