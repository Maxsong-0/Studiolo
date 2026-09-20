use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{AccessError, ApiError, AppState};

pub(crate) async fn guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let head = request.method() == Method::HEAD;
    let bootstrap = request.uri().path() == crate::bootstrap::PATH;
    let allow = match request.uri().path() {
        crate::auth_http::SELF
        | crate::auth_http::FACTOR_STATE
        | crate::bootstrap::PATH
        | crate::canonical::PATH
        | crate::history::PATH
        | crate::history::FILE
        | crate::health::PATH
        | crate::file_operations::TOMBSTONES => "GET",
        crate::auth_http::SESSIONS => "GET, POST",
        crate::auth_http::REVOKE | crate::auth_http::execution::COMMAND => "POST",
        crate::editing::PATH if crate::editing::enabled(&state) => "POST",
        crate::file_operations::PATH if crate::file_operations::available(&state) => "GET, POST",
        path if factors_enabled(&state) && crate::auth_http::is_factor_path(path) => "POST",
        path if state.sessions.is_some() && crate::auth_http::policy::is_policy_path(path) => {
            crate::auth_http::policy::method_for(path).unwrap_or("GET")
        }
        _ => "GET, HEAD",
    };
    let mut response = match authorize(&state, &mut request).await {
        Ok(()) => {
            if let Some(sessions) = &state.sessions
                && request.uri().path().starts_with("/api/")
            {
                let identity = request.extensions_mut().remove::<crate::AccessIdentity>();
                match identity {
                    Some(identity) => match sessions
                        .authorize(&mut request, identity, &state.origin)
                        .await
                    {
                        Ok(()) => next.run(request).await,
                        Err(response) => response,
                    },
                    None => ApiError::unavailable().into_response(),
                }
            } else {
                next.run(request).await
            }
        }
        Err(error) => error.into_response(),
    };
    let method_not_allowed = response.status() == StatusCode::METHOD_NOT_ALLOWED;
    if head && (state.sessions.is_some() || bootstrap) {
        *response.body_mut() = axum::body::Body::empty();
    }
    let nonce = response
        .extensions_mut()
        .remove::<crate::assets::StyleNonce>();
    let style = nonce.map_or_else(
        || "style-src 'self'".to_owned(),
        |nonce| format!("style-src 'self' 'nonce-{}'", nonce.0),
    );
    let headers = response.headers_mut();
    if state.sessions.is_some() {
        headers.insert(header::VARY, "Origin, Sec-Fetch-Site".parse().unwrap());
    }
    if method_not_allowed && (state.sessions.is_some() || bootstrap) {
        headers.insert(header::ALLOW, allow.parse().unwrap());
    }
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    // The opt-in local Consumer companion is bound to literal IPv4 loopback and
    // authenticates both the configured Studiolo origin and a local pairing token.
    // Do not allow arbitrary hosts, localhost DNS aliases, or insecure LAN URLs.
    headers.insert(header::CONTENT_SECURITY_POLICY,
        format!("default-src 'none'; script-src 'self'; {style}; connect-src 'self' http://127.0.0.1:*; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'").parse().unwrap());
    headers.insert(
        "cross-origin-resource-policy",
        "same-origin".parse().unwrap(),
    );
    response
}

fn one<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ApiError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(forbidden());
    }
    value
        .map(|value| value.to_str().map_err(|_| forbidden()))
        .transpose()
}

fn forbidden() -> ApiError {
    ApiError(
        StatusCode::FORBIDDEN,
        "请求不符合当前入口的同源策略。".into(),
    )
}

async fn authorize(state: &AppState, request: &mut Request) -> Result<(), ApiError> {
    let headers = request.headers();
    // 不让代理与应用对重复安全头采用不同解释；即使当前分支不用也先拒绝歧义。
    for name in [
        "host",
        "origin",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-dest",
        "sec-fetch-user",
        "cf-access-jwt-assertion",
    ] {
        one(headers, name)?;
    }
    let host_ok = one(headers, "host")? == Some(state.authority.as_str());
    let origin_ok = one(headers, "origin")?.is_none_or(|value| value == state.origin);
    let site = one(headers, "sec-fetch-site")?;
    let public_navigation = state.access.is_some()
        && request.method() == Method::GET
        && request.uri().path() == "/"
        && request.uri().query().is_none()
        && matches!(site, Some("cross-site" | "same-site"))
        && one(headers, "sec-fetch-mode")? == Some("navigate")
        && one(headers, "sec-fetch-dest")? == Some("document")
        && one(headers, "sec-fetch-user")? == Some("?1");
    let site_ok = matches!(site, None | Some("same-origin" | "none")) || public_navigation;
    if !host_ok || !origin_ok || !site_ok {
        return Err(forbidden());
    }
    if request.uri().path() == crate::bootstrap::PATH {
        if request.method() != Method::GET {
            return Err(ApiError(
                StatusCode::METHOD_NOT_ALLOWED,
                "此引导入口只接受 GET。".into(),
            ));
        }
        if request.uri().query().is_some() {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "引导请求不接受查询参数。".into(),
            ));
        }
    }
    let session_command = state.sessions.is_some()
        && request.method() == Method::POST
        && (matches!(
            request.uri().path(),
            "/api/v1/sessions" | "/api/v1/sessions/revoke"
        ) || (factors_enabled(state)
            && crate::auth_http::is_factor_path(request.uri().path()))
            || (crate::editing::enabled(state) && request.uri().path() == crate::editing::PATH)
            || (crate::file_operations::available(state)
                && request.uri().path() == crate::file_operations::PATH)
            || crate::auth_http::policy::is_policy_post(request.uri().path())
            || request.uri().path() == crate::auth_http::execution::COMMAND);
    if !matches!(*request.method(), Method::GET | Method::HEAD) && !session_command {
        return Err(ApiError(
            StatusCode::METHOD_NOT_ALLOWED,
            "M1 入口只接受只读请求。".into(),
        ));
    }
    if request.uri().to_string().len() > 8192 {
        return Err(ApiError(StatusCode::URI_TOO_LONG, "请求路径过长。".into()));
    }
    if let Some(access) = &state.access {
        let token = one(headers, "cf-access-jwt-assertion")?
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "需要有效的 Access 身份。".into()))?;
        let identity = access.verify(token).await.map_err(|error| {
            let status = if error == AccessError::Unavailable {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::UNAUTHORIZED
            };
            ApiError(status, error.to_string())
        })?;
        request.extensions_mut().insert(identity);
    }
    Ok(())
}

fn factors_enabled(state: &AppState) -> bool {
    state
        .sessions
        .as_ref()
        .is_some_and(crate::auth_http::AuthHttp::has_factors)
}
