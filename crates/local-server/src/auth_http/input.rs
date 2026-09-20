//! 有界流式正文解析；不依赖 Content-Length 或框架 JSON 默认限额。

use std::{future::poll_fn, pin::Pin, time::Duration};

use axum::{
    body::{Body, HttpBody},
    extract::Request,
    http::{Method, StatusCode},
};
use serde::Deserialize;
use studiolo_auth::SessionId;
use zeroize::Zeroizing;

use super::{
    FACTOR_STATE, RECOVER, RECOVERY_CODES, REVOKE, SELF, SESSIONS, STEP_UP, factor_input,
    gate::one, is_factor_path, policy, responses::Failure,
};

pub(super) enum Action {
    Issue,
    Revoke(Revoke),
    Factor(factor_input::Action),
    Content(crate::editing::Replace),
    FileOperation(crate::file_operations::Input),
    Policy(policy::Input),
    Execution(super::execution::Input),
}
pub(super) enum Revoke {
    Current,
    All,
    Session(SessionId),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(tag = "scope", rename_all = "lowercase", deny_unknown_fields)]
// 使用空字段 struct 变体；serde 的内部标记 unit 变体会忽略额外字段。
enum RevokeBody {
    Current {},
    All {},
    Session { session_id: String },
}

pub(super) async fn action(request: &mut Request) -> Result<Option<Action>, Failure> {
    // 只保留固定路径常量，读取正文时不借用可变Request的URI。
    let path = match request.uri().path() {
        SELF => SELF,
        SESSIONS => SESSIONS,
        REVOKE => REVOKE,
        STEP_UP => STEP_UP,
        RECOVERY_CODES => RECOVERY_CODES,
        RECOVER => RECOVER,
        FACTOR_STATE => FACTOR_STATE,
        crate::editing::PATH => crate::editing::PATH,
        crate::file_operations::PATH => crate::file_operations::PATH,
        crate::file_operations::TOMBSTONES => crate::file_operations::TOMBSTONES,
        super::execution::COMMAND => super::execution::COMMAND,
        policy::STATUS => policy::STATUS,
        policy::DESIRED => policy::DESIRED,
        policy::VALIDATE => policy::VALIDATE,
        policy::WITHDRAW => policy::WITHDRAW,
        policy::ACTIVATE => policy::ACTIVATE,
        policy::VERSIONS => policy::VERSIONS,
        policy::REVOKE => policy::REVOKE,
        _ => return Ok(None),
    };
    let posting = request.method() == Method::POST;
    let issuing = path == SESSIONS;
    let headers = request.headers();
    if one(headers, "content-encoding")?.is_some() {
        return Err(Failure::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "认证请求不接受编码正文。",
        ));
    }
    if posting && one(headers, "content-type")? != Some("application/json") {
        return Err(Failure::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "认证请求必须使用 application/json。",
        ));
    }
    let length = one(headers, "content-length")?
        .map(|value| {
            if value.is_empty()
                || value.len() > 10
                || !value.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(Failure::bad_request());
            }
            value.parse::<usize>().map_err(|_| Failure::bad_request())
        })
        .transpose()?;
    if let Some(encoding) = one(headers, "transfer-encoding")?
        && (encoding != "chunked" || length.is_some())
    {
        return Err(Failure::bad_request());
    }
    let limit = if posting && path == crate::editing::PATH {
        crate::editing::JSON_LIMIT
    } else if posting && path == super::execution::COMMAND {
        super::execution::JSON_LIMIT
    } else if posting && policy::is_policy_post(path) {
        policy::JSON_LIMIT
    } else if posting && path == crate::file_operations::PATH {
        crate::file_operations::JSON_LIMIT
    } else if posting {
        1024
    } else {
        0
    };
    if length.is_some_and(|length| length > limit) {
        return Err(too_large());
    }
    let body = std::mem::replace(request.body_mut(), Body::empty());
    let bytes = tokio::time::timeout(Duration::from_secs(2), read(body, limit))
        .await
        .map_err(|_| Failure::new(StatusCode::REQUEST_TIMEOUT, "认证请求正文读取超时。"))??;
    if length.is_some_and(|length| length != bytes.len()) {
        return Err(Failure::bad_request());
    }
    if !posting {
        return Ok(None);
    }
    // serde 的空 struct 也可能接受序列表示；HTTP 明确只接受 JSON 对象。
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(if path == crate::file_operations::PATH {
            Failure::bad_request().with_code("invalid_file_operation")
        } else {
            Failure::bad_request()
        });
    }
    if path == super::execution::COMMAND {
        return super::execution::parse(&bytes).map(|v| Some(Action::Execution(v)));
    }
    if is_factor_path(path) {
        return factor_input::parse(path, &bytes).map(|action| Some(Action::Factor(action)));
    }
    if policy::is_policy_post(path) {
        return policy::parse(path, &bytes).map(|input| Some(Action::Policy(input)));
    }
    if path == crate::editing::PATH {
        return crate::editing::parse(&bytes)
            .map(|input| Some(Action::Content(input)))
            .map_err(|_| Failure::bad_request());
    }
    if path == crate::file_operations::PATH {
        return crate::file_operations::parse(&bytes)
            .map(|input| Some(Action::FileOperation(input)))
            .map_err(|_| Failure::bad_request().with_code("invalid_file_operation"));
    }
    if issuing {
        serde_json::from_slice::<Empty>(&bytes).map_err(|_| Failure::bad_request())?;
        Ok(Some(Action::Issue))
    } else {
        let value =
            serde_json::from_slice::<RevokeBody>(&bytes).map_err(|_| Failure::bad_request())?;
        let value = match value {
            RevokeBody::Current {} => Revoke::Current,
            RevokeBody::All {} => Revoke::All,
            RevokeBody::Session { session_id } => {
                Revoke::Session(SessionId::parse(session_id).map_err(|_| Failure::bad_request())?)
            }
        };
        Ok(Some(Action::Revoke(value)))
    }
}

async fn read(mut body: Body, limit: usize) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(limit));
    let mut frames = 0;
    while let Some(frame) = poll_fn(|context| Pin::new(&mut body).poll_frame(context)).await {
        frames += 1;
        // 连续空帧同样有预算，防止永远 ready 的流阻止全局计时器获得执行机会。
        if frames > 2048 {
            return Err(too_large());
        }
        let data = frame
            .map_err(|_| Failure::bad_request())?
            .into_data()
            .map_err(|_| Failure::bad_request())?;
        if data.len() > limit.saturating_sub(bytes.len()) {
            return Err(too_large());
        }
        bytes.extend_from_slice(&data);
    }
    Ok(bytes)
}

fn too_large() -> Failure {
    Failure::new(StatusCode::PAYLOAD_TOO_LARGE, "认证请求正文超出限制。")
}
