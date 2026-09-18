//! 严格的浏览器请求语法；不采用 Referer、代理来源或宽松旧浏览器降级。

use axum::{
    extract::Request,
    http::{HeaderMap, Method, StatusCode},
};
use studiolo_auth::CsrfToken;
use subtle::ConstantTimeEq;

use super::{
    FACTOR_STATE, REVOKE, SELF, SESSIONS, cookie::Credentials, is_factor_path, policy,
    responses::Failure,
};

pub(super) fn one<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, Failure> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(Failure::forbidden());
    }
    value
        .map(|value| value.to_str().map_err(|_| Failure::forbidden()))
        .transpose()
}

pub(super) fn headers(request: &Request, origin: &str, factors: bool) -> Result<(), Failure> {
    let headers = request.headers();
    let bytes = headers.iter().try_fold(0_usize, |total, (name, value)| {
        total
            .checked_add(name.as_str().len())?
            .checked_add(value.as_bytes().len())?
            .checked_add(4)
    });
    if headers.len() > 128 || bytes.is_none_or(|bytes| bytes > 32 * 1024) {
        return Err(Failure::new(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "认证请求头超出限制。",
        ));
    }
    for name in [
        "host",
        "origin",
        "sec-fetch-site",
        "sec-fetch-mode",
        "sec-fetch-dest",
        "sec-fetch-user",
        "cf-access-jwt-assertion",
        "cookie",
        "x-studiolo-request",
        "x-csrf-token",
        "content-type",
        "content-length",
        "content-encoding",
        "transfer-encoding",
    ] {
        one(headers, name)?;
    }
    if one(headers, "x-studiolo-request")? != Some("1")
        || one(headers, "sec-fetch-site")? != Some("same-origin")
        || !matches!(
            one(headers, "sec-fetch-mode")?,
            Some("cors" | "same-origin")
        )
        || one(headers, "sec-fetch-dest")? != Some("empty")
        || one(headers, "sec-fetch-user")?.is_some()
        || one(headers, "origin")?.is_some_and(|value| value != origin)
        || (request.method() == Method::POST && one(headers, "origin")? != Some(origin))
    {
        return Err(Failure::forbidden());
    }
    let path = request.uri().path();
    let allowed = match path {
        SELF
        | FACTOR_STATE
        | crate::canonical::PATH
        | crate::history::PATH
        | crate::history::FILE
        | crate::file_operations::TOMBSTONES => request.method() == Method::GET,
        SESSIONS => matches!(*request.method(), Method::GET | Method::POST),
        REVOKE => request.method() == Method::POST,
        crate::editing::PATH => request.method() == Method::POST,
        crate::file_operations::PATH => matches!(*request.method(), Method::GET | Method::POST),
        value if factors && is_factor_path(value) => request.method() == Method::POST,
        value if policy::is_policy_path(value) => policy::method_for(value)
            .is_some_and(|allowed| allowed.split(", ").any(|method| method == request.method())),
        _ => matches!(*request.method(), Method::GET | Method::HEAD),
    };
    if !allowed {
        return Err(Failure::method(match path {
            SELF
            | FACTOR_STATE
            | crate::canonical::PATH
            | crate::history::PATH
            | crate::history::FILE
            | crate::file_operations::TOMBSTONES => "GET",
            SESSIONS => "GET, POST",
            REVOKE => "POST",
            crate::editing::PATH => "POST",
            crate::file_operations::PATH => "GET, POST",
            value if factors && is_factor_path(value) => "POST",
            value if policy::is_policy_path(value) => policy::method_for(value).unwrap_or("GET"),
            _ => "GET, HEAD",
        }));
    }
    if (matches!(
        path,
        SELF | SESSIONS
            | REVOKE
            | FACTOR_STATE
            | crate::editing::PATH
            | crate::file_operations::PATH
    ) || (factors && is_factor_path(path))
        || policy::is_policy_path(path))
        && request.uri().query().is_some()
    {
        return Err(Failure::bad_request());
    }
    Ok(())
}

pub(super) fn csrf(headers: &HeaderMap, credentials: &Credentials) -> Result<(), Failure> {
    let value = one(headers, "x-csrf-token")?.ok_or_else(Failure::forbidden)?;
    if value.len() > 84 {
        return Err(Failure::forbidden());
    }
    let supplied = CsrfToken::from_hex(value.to_owned()).map_err(|_| Failure::forbidden())?;
    if !bool::from(
        supplied
            .expose_secret()
            .as_bytes()
            .ct_eq(credentials.csrf.expose_secret().as_bytes()),
    ) {
        return Err(Failure::forbidden());
    }
    Ok(())
}
