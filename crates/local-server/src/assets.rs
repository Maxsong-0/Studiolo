use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use studiolo_core::VaultPath;

use crate::{ApiError, AppState};

pub(crate) const STYLE_NONCE_MARKER: &str = "__STUDIOLO_STYLE_NONCE__";
#[derive(Clone)]
pub(crate) struct StyleNonce(pub(crate) String);

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
    let (content, nonce) = if mime.starts_with("text/html") {
        style_nonce(bytes.content)?
    } else {
        (bytes.content, None)
    };
    let mut response = ([(header::CONTENT_TYPE, mime)], content).into_response();
    if let Some(nonce) = nonce {
        response.extensions_mut().insert(nonce);
    }
    Ok(response)
}

fn style_nonce(bytes: Vec<u8>) -> Result<(Vec<u8>, Option<StyleNonce>), ApiError> {
    let html = String::from_utf8(bytes).map_err(|_| ApiError::unavailable())?;
    match html.matches(STYLE_NONCE_MARKER).count() {
        // 既有旧页面保留原严格CSP；只为含固定占位符的新模板授予动态样式能力。
        0 => Ok((html.into_bytes(), None)),
        1 => {
            let mut random = [0_u8; 32];
            getrandom::fill(&mut random).map_err(|_| ApiError::unavailable())?;
            let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            Ok((
                html.replace(STYLE_NONCE_MARKER, &nonce).into_bytes(),
                Some(StyleNonce(nonce)),
            ))
        }
        _ => Err(ApiError::unavailable()),
    }
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
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        _ => return None,
    };
    Some((relative, mime))
}
