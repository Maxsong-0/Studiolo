//! 附件由真实文件树与 Broker 读取；安全策略不依赖浏览器是否嗅探 MIME。
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use std::fs;
use studiolo_local_server::router;
use studiolo_vault_broker::ReadOnlyVault;
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    vault: TempDir,
    _assets: TempDir,
    app: Router,
}
impl Fixture {
    fn new() -> Self {
        let vault = TempDir::new().unwrap();
        let assets = TempDir::new().unwrap();
        fs::create_dir(vault.path().join("folder")).unwrap();
        fs::create_dir(vault.path().join(".hidden")).unwrap();
        for (name, bytes) in [
            ("file.html", b"<script>alert(1)</script>".as_slice()),
            ("image.png", b"\x89PNG\r\n\x1a\n"),
            ("fake.png", b"<svg onload='alert(1)'/>"),
            ("image.jpg", b"\xff\xd8\xff"),
            ("image.gif", b"GIF89a"),
            ("image.webp", b"RIFF0000WEBP"),
            ("file.pdf", b"%PDF-1.7"),
            ("fake.pdf", b"<html>"),
            ("file.csv", b"<script>,text\n"),
            ("file.canvas", b"{}"),
            ("invalid.txt", b"\xff"),
            ("nul.txt", b"a\0b"),
            ("file.md", b"# markdown"),
            (".hidden/file.txt", b"secret"),
            ("表格 & 说明.csv", b"a,b"),
            ("file.svg", b"<svg/>"),
        ] {
            fs::write(vault.path().join(name), bytes).unwrap();
        }
        fs::File::create(vault.path().join("large.bin"))
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        fs::write(
            assets.path().join("index.html"),
            "<!doctype html><title>附件</title>",
        )
        .unwrap();
        let app = router(
            ReadOnlyVault::open(vault.path()).unwrap(),
            ReadOnlyVault::open(assets.path()).unwrap(),
            "附件夹具".into(),
            4320,
        )
        .unwrap();
        Self {
            vault,
            _assets: assets,
            app,
        }
    }
    async fn request(
        &self,
        path: &str,
        generation: u64,
        method: &str,
        header: Option<(&str, &str)>,
    ) -> Response {
        let encoded: String = path.bytes().map(|byte| format!("%{byte:02X}")).collect();
        let mut request = Request::builder()
            .method(method)
            .uri(format!(
                "/api/attachment?path={encoded}&generation={generation}"
            ))
            .header("host", "127.0.0.1:4320");
        if let Some((key, value)) = header {
            request = request.header(key, value);
        }
        self.app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    async fn get(&self, path: &str) -> Response {
        self.request(path, 1, "GET", None).await
    }
}

#[tokio::test]
async fn extensions_and_bytes_both_constrain_types_and_every_download_is_inert() {
    let f = Fixture::new();
    for (name, mime) in [
        ("file.html", "application/octet-stream"),
        ("file.svg", "application/octet-stream"),
        ("fake.png", "application/octet-stream"),
        ("image.png", "image/png"),
        ("image.jpg", "image/jpeg"),
        ("image.gif", "image/gif"),
        ("image.webp", "image/webp"),
        ("file.csv", "text/plain; charset=utf-8"),
        ("file.canvas", "text/plain; charset=utf-8"),
        ("invalid.txt", "application/octet-stream"),
        ("nul.txt", "application/octet-stream"),
        ("file.pdf", "application/pdf"),
        ("fake.pdf", "application/octet-stream"),
    ] {
        let response = f.get(name).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        let h = response.headers();
        for (key, value) in [
            ("content-type", mime),
            ("cache-control", "no-store, no-transform"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("cross-origin-resource-policy", "same-origin"),
            ("accept-ranges", "none"),
            (
                "content-security-policy",
                "default-src 'none'; sandbox; frame-ancestors 'none'",
            ),
        ] {
            assert_eq!(h[key], value, "{name}: {key}");
        }
        assert!(
            h["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("attachment; filename=")
        );
        let expected = fs::read(f.vault.path().join(name)).unwrap();
        assert_eq!(h["content-length"], expected.len().to_string());
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            expected
        );
    }
}

#[tokio::test]
async fn only_visible_current_attachments_are_readable_and_limits_apply_before_reading() {
    let f = Fixture::new();
    for (path, status) in [
        ("file.md", 404),
        ("folder", 404),
        ("missing.txt", 404),
        (".hidden/file.txt", 400),
        ("../file.csv", 400),
        ("/file.csv", 400),
        ("large.bin", 413),
    ] {
        assert_eq!(f.get(path).await.status().as_u16(), status, "{path}");
    }
    assert_eq!(f.request("file.csv", 0, "GET", None).await.status(), 409);
    fs::write(f.vault.path().join("file.csv"), "changed size").unwrap();
    assert_eq!(f.get("file.csv").await.status(), 409);
    fs::write(f.vault.path().join("file.canvas"), "[]").unwrap();
    let response = f.get("file.canvas").await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"[]"
    );
}

#[tokio::test]
async fn range_is_ignored_head_is_empty_and_methods_origins_and_names_are_safe() {
    let f = Fixture::new();
    let response = f
        .request("file.pdf", 1, "GET", Some(("range", "bytes=0-2")))
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        b"%PDF-1.7"
    );
    let response = f.request("file.pdf", 1, "HEAD", None).await;
    assert_eq!(response.headers()["content-length"], "8");
    assert!(
        to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .is_empty()
    );
    let response = f.request("file.pdf", 1, "POST", None).await;
    assert_eq!(response.status(), 405);
    assert_eq!(response.headers()["allow"], "GET,HEAD");
    assert_eq!(
        f.request(
            "file.pdf",
            1,
            "GET",
            Some(("origin", "https://evil.example"))
        )
        .await
        .status(),
        403
    );
    let response = f.get("表格 & 说明.csv").await;
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"_______.csv\"; filename*=UTF-8''%E8%A1%A8%E6%A0%BC%20%26%20%E8%AF%B4%E6%98%8E.csv"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_a_scanned_attachment_with_symlink_or_hardlink_is_denied() {
    let f = Fixture::new();
    let path = f.vault.path().join("file.csv");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("file.canvas", &path).unwrap();
    assert_eq!(f.get("file.csv").await.status(), 403);
    fs::remove_file(&path).unwrap();
    fs::hard_link(f.vault.path().join("file.canvas"), &path).unwrap();
    assert_eq!(f.get("file.csv").await.status(), 403);
}

#[tokio::test]
async fn slow_downloads_hold_query_budget_until_their_buffers_are_dropped() {
    let f = Fixture::new();
    let first = f.get("file.csv").await;
    let second = f.get("file.pdf").await;
    assert_eq!(first.status(), 200);
    assert_eq!(second.status(), 200);
    assert_eq!(f.get("image.png").await.status(), 429);
    drop(first);
    let third = f.get("image.png").await;
    assert_eq!(third.status(), 200);
    assert_eq!(f.get("file.csv").await.status(), 429);
    to_bytes(second.into_body(), 1024).await.unwrap();
    assert_eq!(f.get("file.csv").await.status(), 200);
    drop(third);
    assert_eq!(f.get("file.pdf").await.status(), 200);
}
