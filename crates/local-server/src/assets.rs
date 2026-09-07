use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use studiolo_core::VaultPath;

use crate::{ApiError, AppState};

pub(crate) async fn asset(
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ApiError> {
    let (path, mime) = asset_path(request.uri().path())
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "资源不存在。".into()))?;
    let path = VaultPath::parse(path)
        .map_err(|_| ApiError(StatusCode::NOT_FOUND, "资源不存在。".into()))?;
    let bytes = tokio::task::spawn_blocking(move || state.assets.read_file(&path, 8 * 1024 * 1024))
        .await
        .map_err(|_| ApiError::unavailable())?
        .map_err(|_| {
            ApiError(
                StatusCode::NOT_FOUND,
                "页面资源不可用，请先构建前端。".into(),
            )
        })?;
    Ok(([(header::CONTENT_TYPE, mime)], bytes.content).into_response())
}

fn asset_path(path: &str) -> Option<(&str, &'static str)> {
    if path == "/" {
        return Some(("index.html", "text/html; charset=utf-8"));
    }
    let relative = path.strip_prefix('/')?;
    if !relative.starts_with("assets/")
        || !relative
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
        || relative
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'))
    {
        return None;
    }
    let mime = match relative.rsplit_once('.')?.1 {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        _ => return None,
    };
    Some((relative, mime))
}
