use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{AccessError, ApiError, AppState};

/// CSP 模板：只有 style-src 随页面 nonce 变化，其余指令固定。可选的本机 Consumer 伴随进程
/// 只绑字面 IPv4 回环，并同时认证已配置的 Studiolo 源与本机配对令牌；因此 connect-src
/// 只放行 `http://127.0.0.1:*`，不放行任意主机、localhost 域名别名或局域网明文地址。
macro_rules! csp {
    ($style:literal) => {
        concat!(
            "default-src 'none'; script-src 'self'; ",
            $style,
            "; connect-src 'self' http://127.0.0.1:*; img-src 'self'; font-src 'self'; ",
            "base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'"
        )
    };
}

/// 无页面 nonce 时的严格策略，也是动态策略不能成为合法头值时的兜底。
const STRICT_CSP: &str = csp!("style-src 'self'");
/// 内容一变文件名就变，旧名永远对应同一份字节，所以可长期缓存且无需再验证。
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_STORE: &str = "no-store";

pub(crate) async fn guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let head = request.method() == Method::HEAD;
    let path = request.uri().path().to_owned();
    let bootstrap = path == crate::bootstrap::PATH;
    let allow = match path.as_str() {
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
    // 先定 CSP：动态策略失败会把响应换成不可用错误，后续状态、HEAD 与缓存判断都以换后的为准。
    let policy = content_security_policy(&mut response);
    let method_not_allowed = response.status() == StatusCode::METHOD_NOT_ALLOWED;
    if head && (state.sessions.is_some() || bootstrap) {
        *response.body_mut() = axum::body::Body::empty();
    }
    // 只有成功送出的文件本体才可能长缓存；错误体即便路径形似哈希资源也一律 no-store。
    let content_type = if response.status() == StatusCode::OK {
        response.headers().get(header::CONTENT_TYPE)
    } else {
        None
    };
    let cache_control = cache_control_for(&path, content_type);
    let headers = response.headers_mut();
    if state.sessions.is_some() {
        headers.insert(
            header::VARY,
            HeaderValue::from_static("Origin, Sec-Fetch-Site"),
        );
    }
    if method_not_allowed && (state.sessions.is_some() || bootstrap) {
        headers.insert(header::ALLOW, HeaderValue::from_static(allow));
    }
    headers.insert(header::CACHE_CONTROL, cache_control);
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, policy);
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// 取出资产处理器留下的样式 nonce 并生成本响应的 CSP。nonce 由本进程生成（十六进制），
/// 若仍不能成为合法头值，就把响应换成不可用错误并配严格策略，绝不 panic。
fn content_security_policy(response: &mut Response) -> HeaderValue {
    let Some(nonce) = response
        .extensions_mut()
        .remove::<crate::assets::StyleNonce>()
    else {
        return HeaderValue::from_static(STRICT_CSP);
    };
    match HeaderValue::from_str(&format!(csp!("style-src 'self' 'nonce-{}'"), nonce.0)) {
        Ok(policy) => policy,
        Err(_) => {
            *response = ApiError::unavailable().into_response();
            HeaderValue::from_static(STRICT_CSP)
        }
    }
}

/// 缓存策略：`/assets/` 下文件名带 Vite 内容哈希（`名字-哈希.js|css|svg|woff2`）、且确以对应
/// MIME 送出的脚本、样式、SVG 图标与 woff2 字体长期缓存；index.html 每次带不同 CSP nonce，
/// 与 API、错误及其余响应一律 no-store。四类共用同一条哈希判定与同一张 MIME 表。
fn cache_control_for(path: &str, content_type: Option<&HeaderValue>) -> HeaderValue {
    let expected = match path
        .strip_prefix("/assets/")
        .and_then(|name| name.rsplit_once('.'))
    {
        Some((stem, "js")) if content_hashed(stem) => crate::assets::JAVASCRIPT,
        Some((stem, "css")) if content_hashed(stem) => crate::assets::STYLESHEET,
        Some((stem, "svg")) if content_hashed(stem) => crate::assets::SVG,
        Some((stem, "woff2")) if content_hashed(stem) => crate::assets::WOFF2,
        _ => return HeaderValue::from_static(NO_STORE),
    };
    if content_type.is_some_and(|value| value == expected) {
        HeaderValue::from_static(IMMUTABLE)
    } else {
        HeaderValue::from_static(NO_STORE)
    }
}

/// Vite 产物名 `[name]-[hash]`：扩展名前是以 `-` 起头、至少 8 位 `[A-Za-z0-9_-]` 的哈希段。
/// 在紧贴扩展名的同字符集连续段里取第一个 `-`，因为哈希本身也可能含 `-`；该段遇 `/` 或 `.` 即止。
fn content_hashed(stem: &str) -> bool {
    let bytes = stem.as_bytes();
    let run = bytes
        .iter()
        .rev()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    let tail = &bytes[bytes.len() - run..];
    tail.iter()
        .position(|byte| *byte == b'-')
        .is_some_and(|dash| tail[dash + 1..].len() >= 8)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 经真实路由只拿得到「类型与扩展名一致」的 200，类型不符的分支只能在这里直接锁住：
    /// 四类长缓存资源各自只认资产处理器给自己的那个 MIME；别类的、缺失或 HTML 都是 no-store。
    #[test]
    fn each_long_cached_type_accepts_only_its_own_mime() {
        let cases = [
            ("/assets/index-C65IIHot.js", crate::assets::JAVASCRIPT),
            ("/assets/index-ovxLtHW2.css", crate::assets::STYLESHEET),
            ("/assets/favicon-4NhN4y5N.svg", crate::assets::SVG),
            ("/assets/archivo-latin-800-normal-DZa_k145.woff2", crate::assets::WOFF2),
        ];
        let html = HeaderValue::from_static("text/html; charset=utf-8");
        for (path, own) in cases {
            let own = HeaderValue::from_static(own);
            assert_eq!(cache_control_for(path, Some(&own)), IMMUTABLE, "{path}");
            assert_eq!(cache_control_for(path, None), NO_STORE, "{path}");
            assert_eq!(cache_control_for(path, Some(&html)), NO_STORE, "{path}");
            for (_, other) in cases {
                let other = HeaderValue::from_static(other);
                if other != own {
                    assert_eq!(cache_control_for(path, Some(&other)), NO_STORE, "{path}");
                }
            }
        }
    }
}
