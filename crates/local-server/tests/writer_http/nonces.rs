//! 实际HTML响应生成专用于样式的随机nonce，不放宽脚本或任意内联样式。
use crate::fixture::Fixture;
use axum::body::to_bytes;
use std::fs;

const TEMPLATE: &str = "<html><head><meta name=\"studiolo-style-nonce\" content=\"__STUDIOLO_STYLE_NONCE__\"></head><body>synthetic</body></html>";

#[tokio::test]
async fn html_style_nonce_is_fresh_per_response_and_matches_only_its_csp() {
    let f = Fixture::new(0, false).await;
    let path = f.http.root.join("web/index.html");
    fs::write(&path, TEMPLATE).unwrap();
    let mut previous = String::new();
    for _ in 0..2 {
        let response = f.http.send(f.http.request("GET", "/", None, ""), 200).await;
        let policy = response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
            .unwrap();
        let nonce = body
            .split("content=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(nonce.len(), 64);
        assert!(
            nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(nonce, previous);
        assert!(policy.contains(&format!("style-src 'self' 'nonce-{nonce}'")));
        assert!(policy.contains("script-src 'self';"));
        assert!(!policy.contains("unsafe-inline") && !body.contains("__STUDIOLO_"));
        previous = nonce.to_owned();
    }
    assert_eq!(fs::read_to_string(path).unwrap(), TEMPLATE);
    let issued = f.http.issue().await;
    let response = f
        .http
        .send(
            f.http
                .request("GET", "/api/workbench", Some(&issued.cookie), ""),
            200,
        )
        .await;
    assert!(
        !response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("nonce-")
    );
    f.close().await;
}

#[tokio::test]
async fn ambiguous_template_never_gets_a_nonce_or_relaxed_policy() {
    let f = Fixture::new(0, false).await;
    fs::write(
        f.http.root.join("web/index.html"),
        format!("{TEMPLATE}{TEMPLATE}"),
    )
    .unwrap();
    let response = f.http.send(f.http.request("GET", "/", None, ""), 503).await;
    assert!(
        !response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("nonce-")
    );
    f.close().await;
}

#[tokio::test]
async fn head_keeps_response_policy_but_does_not_disclose_the_html_body() {
    let f = Fixture::new(0, false).await;
    fs::write(f.http.root.join("web/index.html"), TEMPLATE).unwrap();
    let response = f
        .http
        .send(f.http.request("HEAD", "/", None, ""), 200)
        .await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("style-src 'self' 'nonce-")
    );
    assert!(
        to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .is_empty()
    );
    f.close().await;
}

#[tokio::test]
async fn legacy_html_and_static_assets_keep_their_exact_bytes_and_original_policy() {
    let f = Fixture::new(0, false).await;
    for (uri, file) in [
        ("/", "index.html"),
        ("/assets/app.js", "assets/app.js"),
        ("/assets/app.css", "assets/app.css"),
    ] {
        let expected = fs::read(f.http.root.join("web").join(file)).unwrap();
        let response = f.http.send(f.http.request("GET", uri, None, ""), 200).await;
        let policy = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(policy.contains("style-src 'self';") && !policy.contains("nonce-"));
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
            expected
        );
    }
    f.close().await;
}

#[tokio::test]
async fn identity_and_origin_rejections_never_inherit_a_template_nonce() {
    let f = Fixture::new(0, false).await;
    fs::write(f.http.root.join("web/index.html"), TEMPLATE).unwrap();
    let mut missing_identity = f.http.request("GET", "/", None, "");
    missing_identity
        .headers_mut()
        .remove("cf-access-jwt-assertion");
    let mut foreign_origin = f.http.request("GET", "/", None, "");
    foreign_origin
        .headers_mut()
        .insert("origin", "https://foreign.example".parse().unwrap());
    for (request, status) in [(missing_identity, 401), (foreign_origin, 403)] {
        let response = f.http.send(request, status).await;
        assert!(
            !response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("nonce-")
        );
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
            .unwrap();
        assert!(!body.contains("studiolo-style-nonce"));
    }
    f.close().await;
}
