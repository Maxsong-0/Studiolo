//! Access 浏览器同源策略通过完整 HTTP Interface 验证。

use super::http_tests::{DOCUMENT, ORIGIN, ROUTES, SEARCH, TREE, fixture, reply, request};
use super::test_support::{claims, signed};

#[tokio::test]
async fn public_origin_is_exact_and_api_never_accepts_cross_site_metadata() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for origin in [
        "http://127.0.0.1:4319",
        "https://evil.example",
        "null",
        "http://studiolo.example.com",
        "https://studiolo.example.com/",
    ] {
        let request = request("/api/workbench", Some(&token)).header("origin", origin);
        reply(&app, request, 403).await;
    }
    for site in ["cross-site", "same-site"] {
        for uri in [
            "/api/workbench",
            SEARCH,
            DOCUMENT,
            TREE,
            "/assets/app.js",
            "/assets/favicon-fixture.svg",
        ] {
            let request = request(uri, Some(&token)).header("sec-fetch-site", site);
            reply(&app, request, 403).await;
        }
    }
}

#[tokio::test]
async fn public_exception_only_allows_user_initiated_top_level_home_navigation() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for site in ["cross-site", "same-site"] {
        for (uri, mode, dest, user, expected) in [
            ("/", "navigate", "document", Some("?1"), 200),
            ("/", "navigate", "document", None, 403),
            ("/", "navigate", "iframe", Some("?1"), 403),
            ("/", "cors", "document", Some("?1"), 403),
            ("/?x=1", "navigate", "document", Some("?1"), 403),
            ("/api/workbench", "navigate", "document", Some("?1"), 403),
        ] {
            let mut builder = request(uri, Some(&token))
                .header("sec-fetch-site", site)
                .header("sec-fetch-mode", mode)
                .header("sec-fetch-dest", dest);
            if let Some(user) = user {
                builder = builder.header("sec-fetch-user", user);
            }
            reply(&app, builder, expected).await;
        }
    }
}

#[tokio::test]
async fn every_duplicate_security_header_is_rejected_even_when_values_match() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for (name, value) in [
        ("host", "127.0.0.1:4319"),
        ("origin", ORIGIN),
        ("sec-fetch-site", "same-origin"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-dest", "empty"),
        ("sec-fetch-user", "?1"),
        ("cf-access-jwt-assertion", token.as_str()),
    ] {
        let mut builder = request("/api/workbench", Some(&token));
        if name != "host" && name != "cf-access-jwt-assertion" {
            builder = builder.header(name, value);
        }
        reply(&app, builder.header(name, value), 403).await;
    }
}

#[tokio::test]
async fn write_methods_remain_disabled_and_denials_share_security_headers() {
    let (_root, app) = fixture(true).await;
    let token = signed(&claims());
    for method in ["POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        let request = request("/api/workbench", Some(&token)).method(method);
        reply(&app, request, 405).await;
    }
    let unauthorized = reply(&app, request("/", None), 401).await;
    let request = request("/", Some(&token)).header("origin", "null");
    let forbidden = reply(&app, request, 403).await;
    for name in [
        "cache-control",
        "x-content-type-options",
        "referrer-policy",
        "content-security-policy",
        "cross-origin-resource-policy",
    ] {
        assert_eq!(
            unauthorized.headers().get(name),
            forbidden.headers().get(name)
        );
    }
}

#[tokio::test]
async fn original_local_router_needs_no_identity_and_keeps_strict_browser_guards() {
    let (_root, app) = fixture(false).await;
    for uri in ROUTES {
        let request = request(uri, None).header("origin", "http://127.0.0.1:4319");
        reply(&app, request, 200).await;
    }
    reply(&app, request("/", None).header("origin", ORIGIN), 403).await;
    for site in ["cross-site", "same-site"] {
        let request = request("/", None)
            .header("sec-fetch-site", site)
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-dest", "document")
            .header("sec-fetch-user", "?1");
        reply(&app, request, 403).await;
    }
}
